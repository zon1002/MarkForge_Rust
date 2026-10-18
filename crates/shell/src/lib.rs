//! Editor 與 Viewer 共用、與 Windows 相關的小功能。

pub mod fonts;
pub mod theme;
pub mod webview2;

use std::path::PathBuf;

/// %LOCALAPPDATA%\MarkForge（WebView2 使用者資料等）。
pub fn local_data_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("MarkForge"))
}

/// %APPDATA%\MarkForge（設定檔）。
pub fn roaming_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("MarkForge"))
}

/// 以系統預設程式開啟網址（只允許 http、https、mailto）。
pub fn open_external(url: &str) {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return;
    }
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL);
    }
}

/// 設定環境變數 MARKFORGE_DEBUG_LOG=檔案路徑 時，把訊息附加到該檔（診斷用）。
pub fn debug_log(line: &str) {
    if let Some(path) = std::env::var_os("MARKFORGE_DEBUG_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// URL 路徑的百分比解碼（自訂協定的請求路徑）。
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
