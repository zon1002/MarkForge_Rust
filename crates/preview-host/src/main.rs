//! 檔案總管預覽窗格的渲染程式。由 MarkForgePreview.dll（預覽處理常式）啟動，從不由使用者執行：
//!
//!   MarkForgePreviewHost.exe "<檔案>" <預覽窗格視窗代碼> <left> <top> <right> <bottom>
//!
//! 以 Win32 直接建立預覽窗格的「子視窗」（WS_CHILD、沒有任何外框，視窗類別 `MarkForgePreviewHost`，
//! 處理常式靠這個名稱找到它來調整大小、通知關閉），裡面是 wry（WebView2）顯示渲染後的 Markdown。
//!
//! 選取檔案就會預覽，所以頁面加上嚴格的 Content-Security-Policy：只能載入本程式提供的資源，
//! 遠端圖片、iframe、Markdown 裡的 `<script>` 都不會執行或載入；也不允許頁面跳轉，
//! 點選網址改由預設瀏覽器開啟。失敗時不跳任何對話框，只寫入
//! %LOCALAPPDATA%\MarkForge\PreviewHost\error.log。

#![windows_subsystem = "windows"]

use std::borrow::Cow;
use std::num::NonZeroIsize;
use std::path::{Path, PathBuf};

use markforge_core::import::spreadsheet::{decode_text, system_ansi_encoding};
use markforge_core::render::escape_html;
use markforge_core::{AssetMode, PageOptions, assets, page_html, render_body};
use markforge_shell::{local_data_dir, open_external, percent_decode};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IsWindow, KillTimer, MSG, RegisterClassW,
    SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows::core::{HSTRING, w};
use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

/// 處理常式以這個視窗類別名稱找到本程式的視窗
const WINDOW_CLASS: &str = "MarkForgePreviewHost";
/// 更大的檔案只顯示提示，不渲染
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// 只能載入自己的資源；Markdown 內嵌的 <script>、遠端圖片、iframe 一律擋下。
/// Mermaid 會產生行內樣式，所以 style 允許 'unsafe-inline'。
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data: blob:; font-src 'self' data:; media-src 'self'; connect-src 'self'";
const ORIGIN: &str = "http://markforge.localhost/";
/// 每秒檢查預覽窗格是否還在
const PARENT_CHECK_TIMER: usize = 1;

/// 預覽窗格之下、沒有任何外框的子視窗（直接用 Win32 建立：tao 的視窗即使不顯示外框，
/// 仍保留一圈非工作區，在預覽窗格裡看得到邊線）。
struct HostWindow(HWND);

impl HasWindowHandle for HostWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

static mut PARENT: HWND = HWND(std::ptr::null_mut());

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 處理常式 Unload 時送 WM_CLOSE。沒有東西需要保存；正常釋放 WebView2 太慢
        // （處理常式只等 500ms），直接結束——WebView2 的子程序會自行跟著結束。
        WM_CLOSE => std::process::exit(0),
        // 檔案總管或 prevhost 先結束時沒有人會通知我們；預覽窗格不在了就自行離開
        WM_TIMER if wparam.0 == PARENT_CHECK_TIMER => {
            if !unsafe { IsWindow(Some(PARENT)) }.as_bool() {
                std::process::exit(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_host_window(parent: HWND, l: i32, t: i32, w: i32, h: i32, dark: bool) -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = HSTRING::from(WINDOW_CLASS);
        let background = if dark { 0x0017_110D } else { 0x00FF_FFFF }; // COLORREF 0x00BBGGRR
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            hbrBackground: HBRUSH(CreateSolidBrush(windows::Win32::Foundation::COLORREF(background)).0),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            &class_name,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            l,
            t,
            w,
            h,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 7 {
        return;
    }
    let file = PathBuf::from(&args[1]);
    let (Ok(parent), Ok(l), Ok(t), Ok(r), Ok(b)) = (
        args[2].parse::<isize>(),
        args[3].parse::<i32>(),
        args[4].parse::<i32>(),
        args[5].parse::<i32>(),
        args[6].parse::<i32>(),
    ) else {
        return;
    };
    let parent = HWND(parent as *mut _);
    unsafe { PARENT = parent };
    let dark = windows_uses_dark_apps();

    let hwnd = match create_host_window(parent, l, t, (r - l).max(1), (b - t).max(1), dark) {
        Ok(h) => h,
        Err(e) => return log_error(&format!("window: {e}")),
    };

    let page = build_page(&file, dark);
    let doc_dir = std::path::absolute(&file).ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let mut web_context = WebContext::new(local_data_dir().map(|d| d.join("PreviewHost").join("WebView2")));
    let host = HostWindow(hwnd);
    let webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_custom_protocol("markforge".into(), move |_id, request| serve(&request, &page, doc_dir.as_deref()))
        .with_url(format!("{ORIGIN}index.html"))
        .with_navigation_handler(|url| {
            if url.starts_with(ORIGIN) {
                return true;
            }
            open_external(&url);
            false
        })
        .with_new_window_req_handler(|url, _| {
            open_external(&url);
            wry::NewWindowResponse::Deny
        })
        .with_default_context_menus(true)
        .with_browser_accelerator_keys(false)
        .with_devtools(false)
        .with_focused(false)
        .with_background_color(if dark { (13, 17, 23, 255) } else { (255, 255, 255, 255) })
        .build(&host);
    let _webview = match webview {
        Ok(w) => w,
        Err(e) => return log_error(&format!("WebView2: {e}")),
    };

    unsafe {
        SetTimer(Some(hwnd), PARENT_CHECK_TIMER, 1000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(hwnd), PARENT_CHECK_TIMER);
    }
}

fn build_page(file: &Path, dark: bool) -> String {
    let opts = PageOptions {
        dark,
        // 預覽窗格通常不寬，不限制閱讀寬度
        width_px: 0,
        title: file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        ..PageOptions::default()
    };
    let body = match std::fs::metadata(file) {
        Ok(m) if m.len() > MAX_FILE_BYTES => notice(&format!("檔案過大（{} MB），請以 MarkForge Viewer 開啟。", m.len() / 1024 / 1024)),
        Ok(_) => match std::fs::read(file) {
            Ok(bytes) => render_body(&decode_text(&bytes, system_ansi_encoding())),
            Err(e) => notice(&format!("無法讀取檔案：{e}")),
        },
        Err(e) => notice(&format!("無法讀取檔案：{e}")),
    };
    page_html(&body, &opts, AssetMode::Served)
}

fn notice(text: &str) -> String {
    format!("<p style=\"color: var(--mf-muted); margin-top: 40px\">{}</p>", escape_html(text))
}

fn serve(request: &Request<Vec<u8>>, page: &str, doc_dir: Option<&Path>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    let found: Option<(Cow<'static, [u8]>, &str)> = if path == "/" || path == "/index.html" {
        Some((Cow::Owned(page.as_bytes().to_vec()), "text/html; charset=utf-8"))
    } else if let Some(name) = path.strip_prefix("/assets/") {
        assets::get(name).map(|a| (Cow::Borrowed(a.bytes), a.mime))
    } else if let Some(rel) = percent_decode(path).strip_prefix("/doc/") {
        // 只提供文件資料夾之內的檔案
        doc_dir
            .map(|dir| (dir.join(rel.replace('/', "\\")), dir))
            .filter(|(f, dir)| std::path::absolute(f).is_ok_and(|a| a.starts_with(dir)) && f.is_file())
            .and_then(|(f, _)| std::fs::read(&f).ok().map(|b| (Cow::Owned(b), assets::mime_for_path(&f.to_string_lossy()))))
    } else {
        None
    };
    match found {
        Some((body, mime)) => Response::builder()
            .header(CONTENT_TYPE, mime)
            .header("Content-Security-Policy", CSP)
            .header("Cache-Control", "no-store")
            .body(body)
            .unwrap(),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

/// 跟著 Windows「應用程式模式」深色／淺色（檔案總管本身也是如此）。
fn windows_uses_dark_apps() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    }
    .is_ok();
    ok && value == 0
}

/// %LOCALAPPDATA%\MarkForge\PreviewHost\error.log（超過 256 KB 先清空）。
fn log_error(message: &str) {
    let Some(dir) = local_data_dir().map(|d| d.join("PreviewHost")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("error.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{:?}] {message}", std::time::SystemTime::now());
    }
}
