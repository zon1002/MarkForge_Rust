//! Windows 剪貼簿「HTML Format」（CF_HTML）的處理與來源判斷。

use std::sync::LazyLock;

use regex::Regex;

/// 去掉 CF_HTML 標頭（Version／StartHTML／…／SourceURL），回傳其後的 HTML 文件。
///
/// 標頭裡的位移是 UTF-8 位元組位移，但 Word 會把 `<head>` 內 CSS 的字型名稱以 ANSI
/// 字碼頁寫入，解碼成字串後位移就對不上——照位移切會把樣式表切掉，Word「標題」與內文
/// 的字級全部遺失。因此完全不用位移，直接取標頭之後第一個以 `<` 開頭的行到結尾。
pub fn extract(cf_html: &str) -> &str {
    if !cf_html.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("Version:")) {
        return cf_html;
    }
    let mut offset = 0;
    for line in cf_html.split_inclusive('\n') {
        if line.trim_start_matches([' ', '\t']).starts_with('<') {
            return &cf_html[offset..];
        }
        offset += line.len();
    }
    cf_html
}

/// CF_HTML 原始位元組 → 字串。內容大多是 UTF-8，但 Word 的 `<head>` 可能混有 ANSI
/// 位元組；以有損解碼處理，受影響的只有 CSS 字型名稱。
pub fn decode_cf_html(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

static DOCUMENT_APP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)urn:schemas-microsoft-com:office|content="?(Microsoft Word|Microsoft Excel|LibreOffice|OpenOffice|WPS)|docs-internal-guid|<meta[^>]+generator[^>]+(LibreOffice|OpenOffice)"#,
    )
    .unwrap()
});
static PRE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)white-space\s*:\s*pre").unwrap());
static STRUCTURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<(h[1-6]|table|ul|ol|li|b|strong|em|i|img|a\s|blockquote|pre)\b").unwrap());
static EXCEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)content="?Microsoft Excel|urn:schemas-microsoft-com:office:excel|<meta[^>]+generator[^>]+LibreOffice[^>]*>[\s\S]*<table"#)
        .unwrap()
});

/// 文書軟體（Word、Excel、LibreOffice、WPS、Google 文件）產生的 HTML，值得轉成 Markdown 結構。
pub fn is_from_document_app(html: &str) -> bool {
    DOCUMENT_APP.is_match(html)
}

/// HTML 帶有 Markdown 能表達的結構（標題、清單、表格、強調、圖片、連結），且不是程式碼
/// 編輯器（VS Code 等）的上色片段——後者一律含 `white-space: pre`。
pub fn has_markdown_worthy_structure(html: &str) -> bool {
    !PRE_WHITESPACE.is_match(html) && STRUCTURE.is_match(html)
}

/// 從試算表（Excel、LibreOffice Calc）複製的儲存格。
pub fn is_spreadsheet_html(html: &str) -> bool {
    EXCEL.is_match(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_ignores_offsets_and_keeps_head() {
        let cf = "Version:1.0\r\nStartHTML:0000000999\r\nEndHTML:0000009999\r\nStartFragment:0000000500\r\nEndFragment:0000000600\r\nSourceURL:file:///C:/a.docx\r\n<html><head><style>p.MsoTitle{font-size:28pt}</style></head><body><!--StartFragment--><p>x</p></body></html>";
        let html = extract(cf);
        assert!(html.starts_with("<html><head><style>"));
    }

    #[test]
    fn source_detection() {
        assert!(is_from_document_app(r#"<html xmlns:w="urn:schemas-microsoft-com:office:word">"#));
        assert!(is_from_document_app(r#"<b style="font-weight:normal;" id="docs-internal-guid-abc">"#));
        let vscode = r#"<div style="color: #ccc;font-family: Consolas;white-space: pre;"><div><b>int</b> a;</div></div>"#;
        assert!(!has_markdown_worthy_structure(vscode));
        assert!(has_markdown_worthy_structure("<p>see <a href=\"x\">link</a></p>"));
        assert!(!has_markdown_worthy_structure("<p>plain</p>"));
    }
}
