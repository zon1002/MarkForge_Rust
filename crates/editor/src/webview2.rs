//! wry 沒有包裝、要直接呼叫 WebView2 COM 的功能：
//!   * 拖入檔案的真實路徑：前端以 `chrome.webview.postMessageWithAdditionalObjects` 把 File 物件
//!     傳過來，這裡從 `ICoreWebView2File::Path` 取路徑。（不用 wry 的 drag_drop_handler：它在
//!     Windows 上會整個取代 WebView2 的拖放，編輯區內拖曳文字搬移也會跟著失效。）
//!   * 另存 PDF（`ICoreWebView2_7::PrintToPdf`）。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tao::event_loop::EventLoopProxy;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_7, ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2,
};
use webview2_com::{PrintToPdfCompletedHandler, WebMessageReceivedEventHandler};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::WebViewExtWindows;

use crate::UserEvent;

#[derive(Deserialize)]
struct DropMessage {
    #[serde(rename = "type")]
    kind: String,
    x: f64,
    y: f64,
    #[serde(default)]
    target: String,
}

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
    s
}

/// 註冊 WebMessageReceived：只處理帶有附加物件（拖入的檔案）的訊息，其餘交給 wry 的 IPC。
pub fn install_file_drop_hook(webview: &wry::WebView, proxy: EventLoopProxy<UserEvent>) -> windows::core::Result<()> {
    let core = webview.webview();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else { return Ok(()) };
        let Ok(args2) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>() else { return Ok(()) };
        // 沒有附加物件的一般 IPC 訊息：AdditionalObjects 為空（或回傳錯誤），交給 wry
        let Ok(objects) = (unsafe { args2.AdditionalObjects() }) else { return Ok(()) };
        let mut count = 0u32;
        unsafe { objects.Count(&mut count)? };
        if count == 0 {
            return Ok(());
        }
        // 前端以 JSON 字串送出（帶附加物件時，物件型別的訊息不會送達）
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
            let _ = proxy.send_event(UserEvent::FilesDropped { paths, x: msg.x, y: msg.y, target: msg.target });
        }
        Ok(())
    }));
    let mut token = Default::default();
    unsafe { core.add_WebMessageReceived(&handler, &mut token) }
}

/// 以目前頁面的列印版面輸出 PDF（頁面的 @media print 只留下預覽內容）。完成時送出 `UserEvent::PdfDone`。
pub fn print_to_pdf(webview: &wry::WebView, path: &Path, proxy: EventLoopProxy<UserEvent>) -> windows::core::Result<()> {
    let core7 = webview.webview().cast::<ICoreWebView2_7>()?;
    let target = path.to_path_buf();
    let handler = PrintToPdfCompletedHandler::create(Box::new(move |result, ok| {
        let outcome = match result {
            Ok(()) if ok => Ok(target.clone()),
            Ok(()) => Err("PDF 匯出未成功完成。".to_string()),
            Err(e) => Err(e.message()),
        };
        let _ = proxy.send_event(UserEvent::PdfDone(outcome));
        Ok(())
    }));
    unsafe { core7.PrintToPdf(&HSTRING::from(path.as_os_str()), None, &handler) }
}
