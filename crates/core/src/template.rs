//! 完整 HTML 頁面：Viewer、檔案總管預覽與「匯出為 HTML」使用。
//! （Editor 的預覽窗格是編輯器頁面的一部分，直接取用相同的 CSS／preview.js，不經過這裡。）

use crate::assets;
use crate::render::escape_html;

pub const DEFAULT_FONT_FAMILY: &str = "Segoe UI";
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
pub const DEFAULT_WIDTH_PX: u32 = 860;

/// 頁面外觀設定。
#[derive(Clone, Debug)]
pub struct PageOptions {
    pub dark: bool,
    pub font_family: String,
    pub font_size: f32,
    /// 閱讀欄寬（px）；0 = 不限制，撐滿視窗。
    pub width_px: u32,
    /// 文件自己的相對路徑圖片從哪裡取（例如 `/doc/`）；`None` = 不改寫（匯出檔放在 .md 旁）。
    pub doc_base: Option<String>,
    /// Windows 絕對路徑圖片從哪裡取（例如 `/abs/`）。
    pub abs_base: Option<String>,
    pub title: String,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self {
            dark: true,
            font_family: DEFAULT_FONT_FAMILY.to_string(),
            font_size: DEFAULT_FONT_SIZE,
            width_px: DEFAULT_WIDTH_PX,
            doc_base: Some("/doc/".to_string()),
            abs_base: Some("/abs/".to_string()),
            title: "MarkForge".to_string(),
        }
    }
}

/// 資源載入方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetMode {
    /// `<script src="/assets/…">`，由 WebView 的自訂協定提供（頁面小、載入快）。
    Served,
    /// CSS／JS 全部內嵌，產生可在任何瀏覽器獨立開啟的單一檔案。
    Inline,
}

/// 頁面根元素的 CSS 變數（字型、字級、閱讀寬度）。
pub fn css_variables(opts: &PageOptions) -> String {
    let family = if opts.font_family.trim().is_empty() { DEFAULT_FONT_FAMILY } else { opts.font_family.trim() };
    let width = if opts.width_px == 0 { "none".to_string() } else { format!("{}px", opts.width_px) };
    format!(
        "--mf-font-family: \"{}\", \"Segoe UI\", \"Microsoft JhengHei\", Helvetica, Arial, sans-serif; --mf-font-size: {}px; --mf-width: {};",
        family.replace(['"', '<', '>', ';', '{', '}'], ""),
        opts.font_size,
        width
    )
}

/// 產生完整頁面，`body_html` 為 [`crate::render_body`] 的輸出。
pub fn page_html(body_html: &str, opts: &PageOptions, mode: AssetMode) -> String {
    let theme = if opts.dark { "dark" } else { "light" };
    let hljs_css = if opts.dark { assets::HLJS_DARK_CSS } else { assets::HLJS_LIGHT_CSS };

    let mut head = String::new();
    for css in [hljs_css, assets::MARKDOWN_CSS] {
        head.push_str(&match mode {
            AssetMode::Served => format!("<link rel=\"stylesheet\" href=\"/assets/{css}\">\n"),
            AssetMode::Inline => format!("<style>\n{}\n</style>\n", assets::text(css)),
        });
    }
    for js in [assets::HIGHLIGHT_JS, assets::MERMAID_JS, assets::PREVIEW_JS] {
        head.push_str(&match mode {
            AssetMode::Served => format!("<script src=\"/assets/{js}\"></script>\n"),
            AssetMode::Inline => format!("<script>\n{}\n</script>\n", inline_script(assets::text(js))),
        });
    }

    let js_opts = format!(
        "{{ dark: {}, docBase: {}, absBase: {} }}",
        opts.dark,
        js_string_or_null(opts.doc_base.as_deref()),
        js_string_or_null(opts.abs_base.as_deref())
    );

    format!(
        "<!doctype html>\n<html data-theme=\"{theme}\" style='{vars}'>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}</title>\n\
         <style>html, body {{ margin: 0; background: var(--mf-bg); }}</style>\n{head}</head>\n<body>\n\
         <article id=\"content\" class=\"markdown-body\">\n{body_html}</article>\n\
         <script>\nMarkForgePreview.init({js_opts});\nwindow.mfReady = MarkForgePreview.enhance(document.getElementById('content'), {js_opts});\n</script>\n\
         </body>\n</html>\n",
        vars = css_variables(opts).replace('\'', ""),
        title = escape_html(&opts.title),
    )
}

/// 內嵌腳本中的 `</script` 會提早結束 `<script>` 元素；改寫成 JS 中等價的 `<\/script`。
fn inline_script(js: &str) -> String {
    js.replace("</script", "<\\/script").replace("</SCRIPT", "<\\/SCRIPT")
}

fn js_string_or_null(value: Option<&str>) -> String {
    match value {
        None => "null".to_string(),
        Some(v) => {
            let mut s = String::from("'");
            for ch in v.chars() {
                match ch {
                    '\'' => s.push_str("\\'"),
                    '\\' => s.push_str("\\\\"),
                    '<' => s.push_str("\\u003c"),
                    _ => s.push(ch),
                }
            }
            s.push('\'');
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_body;

    #[test]
    fn served_page_references_assets() {
        let html = page_html(&render_body("# Hi"), &PageOptions::default(), AssetMode::Served);
        assert!(html.contains("<html data-theme=\"dark\""));
        assert!(html.contains("src=\"/assets/mermaid.min.js\""));
        assert!(html.contains("href=\"/assets/github-dark.min.css\""));
        assert!(html.contains("docBase: '/doc/'"));
        assert!(html.len() < 20_000);
    }

    #[test]
    fn inline_page_embeds_everything_and_keeps_relative_images() {
        let opts = PageOptions { dark: false, doc_base: None, abs_base: None, ..PageOptions::default() };
        let html = page_html(&render_body("![a](pic.png)"), &opts, AssetMode::Inline);
        assert!(!html.contains("/assets/"));
        assert!(html.contains("docBase: null"));
        assert!(html.contains("src=\"pic.png\""));
        assert!(html.len() > 3_000_000, "mermaid.js should be inlined");
        // 內嵌腳本不可出現會提早結束 <script> 的字串（只有我們自己的結束標籤）
        let closing = html.matches("</script>").count();
        assert_eq!(closing, 4, "3 inline scripts + 1 bootstrap");
    }

    #[test]
    fn width_zero_means_unlimited() {
        let opts = PageOptions { width_px: 0, ..PageOptions::default() };
        assert!(css_variables(&opts).contains("--mf-width: none"));
    }
}
