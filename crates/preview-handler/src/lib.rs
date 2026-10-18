//! MarkForge 檔案總管預覽處理常式（IPreviewHandler）。
//!
//! 檔案總管把預覽處理常式載入共用的 prevhost.exe 代理程序。這個 DLL 刻意保持輕量：只實作檔案總管
//! 呼叫的 COM 介面，在 DoPreview 時啟動同資料夾的 MarkForgePreviewHost.exe，把檔案路徑、預覽窗格
//! 的視窗代碼與範圍交給它；渲染（WebView2）在獨立程序中進行，當掉也不會影響檔案總管。之後由
//! SetRect 調整它的視窗大小、Unload 時關閉它。
//!
//! 以 IInitializeWithFile 取得檔案路徑（預覽要從文件所在資料夾讀取相對路徑圖片），因此登錄時要設
//! DisableLowILProcessIsolation=1；WebView2 也無法在低完整性程序中執行。
//!
//! 必須註冊在 HKLM：COM 不會在 prevhost.exe 代理程序中啟用只註冊在 HKCU 的類別（C# 版 v1.7.0 實測：
//! CoCreateInstance 回傳 REGDB_E_CLASSNOTREG，檔案總管只顯示「沒有預覽可用」）。

#![allow(non_snake_case)]
// COM 方法的簽章由 windows-rs 的 *_Impl trait 決定（指標參數、不能標 unsafe）；呼叫端是 COM
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, CloseHandle, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER,
    E_UNEXPECTED, HANDLE, HINSTANCE, HWND, LPARAM, RECT, S_FALSE, S_OK, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::{
    CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithFile_Impl};
use windows::Win32::UI::Shell::{
    IPreviewHandler, IPreviewHandler_Impl, IPreviewHandlerFrame, IPreviewHandlerVisuals, IPreviewHandlerVisuals_Impl,
    SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
};
use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, MSG, PostMessageW, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLOSE,
};
use windows::core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement, w};

pub mod registry;

// {57C7730F-A427-425D-92B1-CE2797C1C9CA}
pub const CLSID_MARKFORGE_PREVIEW: GUID = GUID::from_u128(0x57C7730F_A427_425D_92B1_CE2797C1C9CA);
pub const HOST_EXE: &str = "MarkForgePreviewHost.exe";

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static OBJECTS: AtomicI32 = AtomicI32::new(0);
static LOCKS: AtomicI32 = AtomicI32::new(0);

fn module() -> HINSTANCE {
    HINSTANCE(MODULE.load(Ordering::Relaxed))
}

/// 本 DLL 的完整路徑。
pub(crate) fn module_path() -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(module().into()), &mut buf) } as usize;
    (n > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// 執行 COM 方法本體；panic 不可越過 COM 邊界，轉成 E_FAIL。
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_FAIL.into()))
}

// ============================== 預覽處理常式 ==============================

#[derive(Default)]
struct State {
    file: Option<String>,
    parent: HWND,
    rect: RECT,
    site: Option<IUnknown>,
    process: HANDLE,
}

#[implement(IPreviewHandler, IPreviewHandlerVisuals, IInitializeWithFile, IObjectWithSite, IOleWindow)]
pub struct MarkdownPreviewHandler {
    st: RefCell<State>,
}

impl Default for MarkdownPreviewHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownPreviewHandler {
    pub fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self { st: RefCell::new(State::default()) }
    }

    /// 預覽程式的視窗：預覽窗格之下、類別名稱為 MarkForgePreviewHost 的子視窗。
    fn host_window(&self) -> Option<HWND> {
        let parent = self.st.borrow().parent;
        if parent.is_invalid() {
            return None;
        }
        unsafe { FindWindowExW(Some(parent), None, w!("MarkForgePreviewHost"), PCWSTR::null()) }.ok()
    }

    fn position_host(&self) {
        let rect = self.st.borrow().rect;
        if let Some(host) = self.host_window() {
            unsafe {
                let _ = SetWindowPos(
                    host,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// 請預覽程式結束（WM_CLOSE）；500ms 內沒結束就強制終止，不讓它拖慢檔案總管切換檔案。
    fn stop_host(&self) {
        let process = std::mem::take(&mut self.st.borrow_mut().process);
        if process.is_invalid() {
            return;
        }
        unsafe {
            match self.host_window() {
                Some(host) => {
                    let _ = PostMessageW(Some(host), WM_CLOSE, WPARAM(0), LPARAM(0));
                    if WaitForSingleObject(process, 500) == WAIT_TIMEOUT {
                        let _ = TerminateProcess(process, 0);
                    }
                }
                None => {
                    let _ = TerminateProcess(process, 0);
                }
            }
            let _ = CloseHandle(process);
        }
    }
}

impl Drop for MarkdownPreviewHandler {
    fn drop(&mut self) {
        self.stop_host();
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IInitializeWithFile_Impl for MarkdownPreviewHandler_Impl {
    fn Initialize(&self, path: &PCWSTR, _mode: u32) -> Result<()> {
        guard(|| {
            if path.is_null() {
                return Err(E_INVALIDARG.into());
            }
            self.st.borrow_mut().file = Some(unsafe { path.to_string() }.map_err(|_| E_INVALIDARG)?);
            Ok(())
        })
    }
}

impl IPreviewHandler_Impl for MarkdownPreviewHandler_Impl {
    fn SetWindow(&self, parent: HWND, prc: *const RECT) -> Result<()> {
        guard(|| {
            if parent.is_invalid() || prc.is_null() {
                return Err(E_INVALIDARG.into());
            }
            {
                let mut st = self.st.borrow_mut();
                st.parent = parent;
                st.rect = unsafe { *prc };
            }
            self.position_host();
            Ok(())
        })
    }

    fn SetRect(&self, prc: *const RECT) -> Result<()> {
        guard(|| {
            if prc.is_null() {
                return Err(E_INVALIDARG.into());
            }
            self.st.borrow_mut().rect = unsafe { *prc };
            self.position_host();
            Ok(())
        })
    }

    fn DoPreview(&self) -> Result<()> {
        guard(|| {
            self.stop_host();
            let (file, parent, rect) = {
                let st = self.st.borrow();
                (st.file.clone(), st.parent, st.rect)
            };
            let Some(file) = file else { return Err(E_UNEXPECTED.into()) };
            if parent.is_invalid() {
                return Err(E_UNEXPECTED.into());
            }
            let exe = module_path().and_then(|p| p.parent().map(|d| d.join(HOST_EXE))).ok_or(E_FAIL)?;

            // "host.exe" "<file>" <hwnd> <left> <top> <right> <bottom>
            let command = format!(
                "\"{}\" \"{}\" {} {} {} {} {}",
                exe.display(),
                file,
                parent.0 as isize,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
            let mut command: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
            let mut pi = PROCESS_INFORMATION::default();
            unsafe {
                CreateProcessW(
                    &HSTRING::from(exe.as_os_str()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    false,
                    PROCESS_CREATION_FLAGS(0),
                    None,
                    PCWSTR::null(),
                    &si,
                    &mut pi,
                )?;
                let _ = CloseHandle(pi.hThread);
            }
            self.st.borrow_mut().process = pi.hProcess;
            Ok(())
        })
    }

    fn Unload(&self) -> Result<()> {
        guard(|| {
            self.stop_host();
            self.st.borrow_mut().file = None;
            Ok(())
        })
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn QueryFocus(&self) -> Result<HWND> {
        let focused = unsafe { GetFocus() };
        if focused.is_invalid() { Err(E_FAIL.into()) } else { Ok(focused) }
    }

    fn TranslateAccelerator(&self, msg: *const MSG) -> Result<()> {
        // 讓檔案總管處理它自己的快速鍵（例如 Tab 離開預覽窗格）
        let site = self.st.borrow().site.clone();
        match site.and_then(|s| s.cast::<IPreviewHandlerFrame>().ok()) {
            Some(frame) => unsafe { frame.TranslateAccelerator(msg) },
            None => Err(S_FALSE.into()),
        }
    }
}

// 檔案總管傳來的窗格配色：預覽程式跟著 Windows 深色／淺色設定（檔案總管本身也是），這裡不使用
impl IPreviewHandlerVisuals_Impl for MarkdownPreviewHandler_Impl {
    fn SetBackgroundColor(&self, _color: windows::Win32::Foundation::COLORREF) -> Result<()> {
        Ok(())
    }
    fn SetFont(&self, _font: *const LOGFONTW) -> Result<()> {
        Ok(())
    }
    fn SetTextColor(&self, _color: windows::Win32::Foundation::COLORREF) -> Result<()> {
        Ok(())
    }
}

impl IObjectWithSite_Impl for MarkdownPreviewHandler_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> Result<()> {
        self.st.borrow_mut().site = site.cloned();
        Ok(())
    }
    fn GetSite(&self, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        let site = self.st.borrow().site.clone();
        match site {
            Some(s) => unsafe { s.query(riid, ppv).ok() },
            None => Err(E_FAIL.into()),
        }
    }
}

impl IOleWindow_Impl for MarkdownPreviewHandler_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        let parent = self.st.borrow().parent;
        if parent.is_invalid() { Err(E_FAIL.into()) } else { Ok(parent) }
    }
    fn ContextSensitiveHelp(&self, _enter: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

// ============================== Class factory ==============================

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        guard(|| unsafe {
            if ppv.is_null() || riid.is_null() {
                return Err(E_POINTER.into());
            }
            *ppv = std::ptr::null_mut();
            if !outer.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let obj: IUnknown = MarkdownPreviewHandler::new().into();
            obj.query(riid, ppv).ok()
        })
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

// ============================== DLL 匯出 ==============================

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(instance.0, Ordering::Relaxed);
    }
    BOOL(1)
}

/// # Safety
/// 由 COM 呼叫；指標參數須符合 DllGetClassObject 規範。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    catch_unwind(AssertUnwindSafe(|| unsafe {
        if ppv.is_null() {
            return E_POINTER;
        }
        *ppv = std::ptr::null_mut();
        if clsid.is_null() || riid.is_null() {
            return E_POINTER;
        }
        if *clsid != CLSID_MARKFORGE_PREVIEW {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = ClassFactory.into();
        factory.query(riid, ppv)
    }))
    .unwrap_or(E_FAIL)
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

/// regsvr32（系統管理員）：寫入與安裝程式相同的 HKLM 機碼。
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    let hr = match registry::register() {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    };
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    hr
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    registry::unregister();
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    S_OK
}
