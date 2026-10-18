//! wry 沒有包裝、要直接呼叫 WebView2 COM 的功能：
//!   * 拖入檔案的真實路徑：前端以 `chrome.webview.postMessageWithAdditionalObjects` 把 File 物件
//!     傳過來，這裡從 `ICoreWebView2File::Path` 取路徑。不用 wry 的 drag_drop_handler：它在
//!     Windows 上會整個取代 WebView2 的拖放，頁面內拖曳文字（例如編輯區搬移選取文字）會跟著失效。
//!     訊息本身必須是 JSON **字串**——帶附加物件時，物件型別的訊息 WebView2 不會送達（實測）。
//!   * 另存 PDF（`ICoreWebView2_7::PrintToPdf`，以頁面的 @media print 版面輸出）。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_7, ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2,
};
use webview2_com::{PrintToPdfCompletedHandler, WebMessageReceivedEventHandler};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::WebViewExtWindows;

/// 拖入頁面的檔案。`x`、`y` 為頁面座標（CSS px），`target` 由前端指定（例如 "editor"）。
#[derive(Debug, Clone)]
pub struct DroppedFiles {
    pub paths: Vec<PathBuf>,
    pub x: f64,
    pub y: f64,
    pub target: String,
}

/// 前端送出的拖放訊息：`JSON.stringify({ type: 'drop', x, y, target })`。
#[derive(Deserialize)]
struct DropMessage {
    #[serde(rename = "type")]
    kind: String,
    x: f64,
    y: f64,
    #[serde(default)]
    target: String,
}

/// 前端拖放訊息的開頭；wry 的 IPC 也會收到同一個字串，應用程式據此略過。
pub const DROP_MESSAGE_PREFIX: &str = r#"{"type":"drop""#;

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
    s
}

/// 註冊 WebMessageReceived：只處理帶有附加物件（拖入的檔案）的訊息，其餘留給 wry 的 IPC。
pub fn install_file_drop_hook(
    webview: &wry::WebView,
    on_drop: impl Fn(DroppedFiles) + 'static,
) -> windows::core::Result<()> {
    let core = webview.webview();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else { return Ok(()) };
        let Ok(args2) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>() else { return Ok(()) };
        // 一般 IPC 訊息沒有附加物件（AdditionalObjects 為空或回傳錯誤）
        let Ok(objects) = (unsafe { args2.AdditionalObjects() }) else { return Ok(()) };
        let mut count = 0u32;
        unsafe { objects.Count(&mut count)? };
        if count == 0 {
            return Ok(());
        }
        let mut text = PWSTR::null();
        unsafe { args.TryGetWebMessageAsString(&mut text)? };
        let Ok(msg) = serde_json::from_str::<DropMessage>(&take_pwstr(text)) else { return Ok(()) };
        if msg.kind != "drop" {
            return Ok(());
        }
        let mut paths = Vec::new();
        for i in 0..count {
            let Ok(obj) = (unsafe { objects.GetValueAtIndex(i) }) else { continue };
            let Ok(file) = obj.cast::<ICoreWebView2File>() else { continue };
            let mut p = PWSTR::null();
            if unsafe { file.Path(&mut p) }.is_ok() {
                let path = take_pwstr(p);
                if !path.is_empty() {
                    paths.push(PathBuf::from(path));
                }
            }
        }
        if !paths.is_empty() {
            on_drop(DroppedFiles { paths, x: msg.x, y: msg.y, target: msg.target });
        }
        Ok(())
    }));
    let mut token = Default::default();
    unsafe { core.add_WebMessageReceived(&handler, &mut token) }
}

/// 以目前頁面的列印版面輸出 PDF，完成（或失敗）時呼叫 `on_done`。
pub fn print_to_pdf(
    webview: &wry::WebView,
    path: &Path,
    on_done: impl FnOnce(Result<PathBuf, String>) + 'static,
) -> windows::core::Result<()> {
    let core7 = webview.webview().cast::<ICoreWebView2_7>()?;
    let target = path.to_path_buf();
    let handler = PrintToPdfCompletedHandler::create(Box::new(move |result, ok| {
        on_done(match result {
            Ok(()) if ok => Ok(target),
            Ok(()) => Err("PDF 匯出未成功完成。".to_string()),
            Err(e) => Err(e.message()),
        });
        Ok(())
    }));
    unsafe { core7.PrintToPdf(&HSTRING::from(path.as_os_str()), None, &handler) }
}
