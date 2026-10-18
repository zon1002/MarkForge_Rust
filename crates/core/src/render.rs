//! Markdown → HTML 片段。
//!
//! 與 C# 版（Markdig `UseAdvancedExtensions`）對齊的 GFM 擴充：表格、刪除線、任務清單、
//! 自動連結、註腳、定義清單、emoji 短碼（`:smile:`）、標題自動 id。另外：
//!   * 單一換行即換行（`hardbreaks`）——一般使用者的直覺，Word 貼上的段內換行也得以保留
//!   * 允許原始 HTML（與 Markdig 預設相同）
//!   * 每個區塊元素帶 `data-sourcepos="起始行:欄-結束行:欄"`，供編輯區／預覽區同步捲動對位
//!   * ```` ```mermaid ```` 區塊輸出為 `<pre class="mermaid">`，由 mermaid.js 在頁面上畫成圖

use comrak::nodes::{AstNode, NodeHtmlBlock, NodeValue};
use comrak::{Arena, Options, format_html, parse_document};

/// 本程式使用的 comrak 選項。
pub fn options() -> Options<'static> {
    let mut o = Options::default();
    o.extension.strikethrough = true;
    o.extension.table = true;
    o.extension.autolink = true;
    o.extension.tasklist = true;
    o.extension.footnotes = true;
    o.extension.description_lists = true;
    o.extension.shortcodes = true;
    o.extension.header_id_prefix = Some(String::new());
    // GitHub 的 > [!NOTE] 提示框
    o.extension.alerts = true;
    // 中文緊鄰標點時 **粗體**、*斜體* 依然成立（CommonMark 原規則對 CJK 不友善）
    o.extension.cjk_friendly_emphasis = true;
    o.render.tasklist_classes = true;
    o.render.hardbreaks = true;
    o.render.r#unsafe = true;
    o.render.sourcepos = true;
    o
}

/// 把 Markdown 轉成 HTML 片段（不含 `<html>`／`<body>`）。
pub fn render_body(markdown: &str) -> String {
    let arena = Arena::new();
    let opts = options();
    let root = parse_document(&arena, markdown, &opts);
    convert_mermaid_blocks(root);

    let mut html = String::new();
    // 寫入 String 不會失敗
    let _ = format_html(root, &opts, &mut html);
    html
}

/// ```` ```mermaid ```` 程式碼區塊改為原始 HTML 的 `<pre class="mermaid">`，內容維持
/// HTML 跳脫，並自行補上 `data-sourcepos`（comrak 不會替原始 HTML 區塊加這個屬性）。
fn convert_mermaid_blocks<'a>(root: &'a AstNode<'a>) {
    for node in root.descendants() {
        let mut ast = node.data.borrow_mut();
        let NodeValue::CodeBlock(block) = &ast.value else { continue };
        let lang = block.info.split_whitespace().next().unwrap_or("");
        if !lang.eq_ignore_ascii_case("mermaid") {
            continue;
        }
        let sp = ast.sourcepos;
        let literal = format!(
            "<pre class=\"mermaid\" data-sourcepos=\"{}:{}-{}:{}\">{}</pre>\n",
            sp.start.line,
            sp.start.column,
            sp.end.line,
            sp.end.column,
            escape_html(&block.literal)
        );
        ast.value = NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 0, literal });
    }
}

pub(crate) fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gfm_table_tasklist_strikethrough() {
        let html = render_body("| a | b |\n| - | -: |\n| 1 | 2 |\n\n- [x] 完成\n- [ ] 未完成\n\n~~刪除~~\n");
        assert!(html.contains("<table"), "{html}");
        assert!(html.contains("align=\"right\"") || html.contains("text-align: right"), "{html}");
        assert!(html.contains("type=\"checkbox\""), "{html}");
        assert!(html.contains(">刪除</del>"), "{html}");
    }

    #[test]
    fn mermaid_block_becomes_pre_mermaid_with_sourcepos() {
        let html = render_body("# 標題\n\n```mermaid\ngraph TD\n  A --> B\n```\n");
        assert!(html.contains("<pre class=\"mermaid\" data-sourcepos=\"3:1-6:3\">graph TD\n  A --&gt; B\n</pre>"), "{html}");
        assert!(!html.contains("language-mermaid"), "{html}");
    }

    #[test]
    fn other_code_blocks_keep_language_class() {
        let html = render_body("```rust\nfn main() {}\n```\n");
        assert!(html.contains("class=\"language-rust\""), "{html}");
    }

    #[test]
    fn block_elements_carry_sourcepos() {
        let html = render_body("# H\n\npara\n");
        assert!(html.contains("<h1 id=\"h\" data-sourcepos=\"1:1-1:3\">"), "{html}");
        assert!(html.contains("<p data-sourcepos=\"3:1-3:4\">"), "{html}");
    }

    #[test]
    fn single_newline_is_a_line_break() {
        let html = render_body("第一行\n第二行\n");
        assert!(html.contains("<br"), "{html}");
    }

    #[test]
    fn emoji_shortcodes_and_raw_html() {
        let html = render_body(":smile: <kbd>Ctrl</kbd>\n");
        assert!(html.contains('😄'), "{html}");
        assert!(html.contains("<kbd>Ctrl</kbd>"), "{html}");
    }
}
