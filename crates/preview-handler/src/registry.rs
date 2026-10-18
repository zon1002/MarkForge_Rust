//! 預覽處理常式的登錄機碼（HKLM；與 installer.iss 寫入的內容相同）。

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, REG_DWORD, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::module_path;

pub const CLSID_STRING: &str = "{57C7730F-A427-425D-92B1-CE2797C1C9CA}";
pub const HANDLER_NAME: &str = "MarkForge Markdown Preview Handler";
/// IPreviewHandler 的 ShellEx 機碼
const PREVIEW_SHELLEX: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";
/// 64 位元預覽處理常式的 prevhost.exe 代理程序
const PREVHOST_APPID: &str = "{6d2b5079-2f0b-48dd-ab7f-97cec514d30b}";
const EXTENSIONS: &[&str] = &[".md", ".markdown"];
const PREVIEW_HANDLERS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers";

fn set_string(root: HKEY, key: &str, name: Option<&str>, value: &str) -> Result<()> {
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let name = name.map(HSTRING::from);
    let status = unsafe {
        RegSetKeyValueW(
            root,
            &HSTRING::from(key),
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    status.ok()
}

fn set_dword(root: HKEY, key: &str, name: &str, value: u32) -> Result<()> {
    let status = unsafe {
        RegSetKeyValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_DWORD.0,
            Some((&value as *const u32).cast()),
            4,
        )
    };
    status.ok()
}

pub fn register() -> Result<()> {
    let dll = module_path().ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    let classes = r"SOFTWARE\Classes";
    let clsid_key = format!(r"{classes}\CLSID\{CLSID_STRING}");
    let root = HKEY_LOCAL_MACHINE;
    set_string(root, &clsid_key, None, HANDLER_NAME)?;
    set_string(root, &clsid_key, Some("DisplayName"), HANDLER_NAME)?;
    set_string(root, &clsid_key, Some("AppID"), PREVHOST_APPID)?;
    // 預覽程式需要 WebView2，無法在低完整性程序中執行
    set_dword(root, &clsid_key, "DisableLowILProcessIsolation", 1)?;
    set_string(root, &format!(r"{clsid_key}\InprocServer32"), None, &dll.to_string_lossy())?;
    set_string(root, &format!(r"{clsid_key}\InprocServer32"), Some("ThreadingModel"), "Apartment")?;
    for ext in EXTENSIONS {
        set_string(root, &format!(r"{classes}\{ext}\ShellEx\{PREVIEW_SHELLEX}"), None, CLSID_STRING)?;
    }
    set_string(root, PREVIEW_HANDLERS, Some(CLSID_STRING), HANDLER_NAME)
}

/// 移除登錄；副檔名的預覽處理常式只在仍是本程式時才刪除（可能已被其他程式接手）。
pub fn unregister() {
    let classes = r"SOFTWARE\Classes";
    let root = HKEY_LOCAL_MACHINE;
    for ext in EXTENSIONS {
        let key = HSTRING::from(format!(r"{classes}\{ext}\ShellEx\{PREVIEW_SHELLEX}"));
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let ours = unsafe {
            RegGetValueW(root, &key, PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
        } == ERROR_SUCCESS
            && String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]).eq_ignore_ascii_case(CLSID_STRING);
        if ours {
            unsafe {
                let _ = RegDeleteTreeW(root, &key);
            }
        }
    }
    unsafe {
        let _ = RegDeleteTreeW(root, &HSTRING::from(format!(r"{classes}\CLSID\{CLSID_STRING}")));
        let _ = RegDeleteKeyValueW(root, &HSTRING::from(PREVIEW_HANDLERS), &HSTRING::from(CLSID_STRING));
    }
}
