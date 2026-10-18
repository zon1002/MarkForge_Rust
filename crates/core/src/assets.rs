//! 內嵌的離線資源。WebView 內的頁面以 `/assets/<名稱>` 取用（各程式的自訂協定處理常式
//! 呼叫 [`get`]）；匯出獨立 HTML 時則直接內嵌文字內容。

pub struct Asset {
    pub bytes: &'static [u8],
    pub mime: &'static str,
}

macro_rules! asset {
    ($file:literal, $mime:literal) => {
        Asset { bytes: include_bytes!(concat!("../assets/", $file)), mime: $mime }
    };
}

pub const HIGHLIGHT_JS: &str = "highlight.min.js";
pub const MERMAID_JS: &str = "mermaid.min.js";
pub const PREVIEW_JS: &str = "preview.js";
pub const MARKDOWN_CSS: &str = "markdown.css";
pub const HLJS_LIGHT_CSS: &str = "github.min.css";
pub const HLJS_DARK_CSS: &str = "github-dark.min.css";

/// 依檔名取得資源；不認得的名稱回傳 `None`。
pub fn get(name: &str) -> Option<Asset> {
    Some(match name {
        HIGHLIGHT_JS => asset!("highlight.min.js", "text/javascript"),
        MERMAID_JS => asset!("mermaid.min.js", "text/javascript"),
        PREVIEW_JS => asset!("preview.js", "text/javascript"),
        MARKDOWN_CSS => asset!("markdown.css", "text/css"),
        HLJS_LIGHT_CSS => asset!("github.min.css", "text/css"),
        HLJS_DARK_CSS => asset!("github-dark.min.css", "text/css"),
        "editor.ico" => asset!("editor.ico", "image/x-icon"),
        "viewer.ico" => asset!("viewer.ico", "image/x-icon"),
        _ => return None,
    })
}

/// 文字資源（內嵌進 HTML 用）。
pub fn text(name: &str) -> &'static str {
    let asset = get(name).unwrap_or_else(|| panic!("未知的資源：{name}"));
    std::str::from_utf8(asset.bytes).expect("文字資源必須是 UTF-8")
}

/// 依副檔名推測 Content-Type（提供文件圖片、附件給 WebView 時使用）。
pub fn mime_for_path(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "tif" | "tiff" => "image/tiff",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "md" | "markdown" | "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_named_assets_exist() {
        for name in [HIGHLIGHT_JS, MERMAID_JS, PREVIEW_JS, MARKDOWN_CSS, HLJS_LIGHT_CSS, HLJS_DARK_CSS] {
            assert!(!text(name).is_empty(), "{name}");
        }
        assert!(get("nope.js").is_none());
    }
}
