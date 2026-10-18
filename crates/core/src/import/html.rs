//! 剪貼簿 HTML（Word、LibreOffice Writer、Google 文件、網頁）→ GFM Markdown。
//!
//! 標題有兩個來源：
//!   * 真正的 `<h1>`～`<h6>`（Word 的「標題 1～6」、LibreOffice 的 Heading 樣式）
//!   * 字體明顯比內文大的一般段落（手動放大的標題、Word 的 MsoTitle、Google 文件的 Title）。
//!     內文字級 = 貼上內容中「字元數最多」的字級；每個段落的字級 = 該段落中字元數最多的字級，
//!     所以段首一個放大的字不會讓整段變成標題。
//!
//! Word 的清單段落（`style="mso-list:l0 level2"`，項目符號以文字寫在
//! `<![if !supportLists]>` 區塊內）會重建為巢狀 Markdown 清單；HTML 的 ul／ol 與表格照常轉換。

use std::collections::HashMap;
use std::sync::LazyLock;

use ego_tree::NodeRef;
use regex::Regex;
use scraper::{Html, Node};

use super::table;

/// 只有一種字級時假設的內文大小（Word／Writer 常見內文），讓單獨貼上的 24pt 標題仍會成為標題。
const DEFAULT_BODY_PT: f64 = 12.0;

const BLOCK_TAGS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "table", "pre", "blockquote", "hr", "section",
    "article", "header", "footer", "main", "nav", "aside", "figure", "figcaption", "dl", "dt", "dd", "address",
    "center", "body", "html", "form", "fieldset", "details", "summary", "tbody", "thead", "tfoot", "tr",
];
const SKIP_TAGS: &[&str] = &[
    "head", "style", "script", "title", "meta", "link", "noscript", "template", "svg", "button", "input", "select",
    "textarea", "iframe", "object", "xml", "colgroup", "col",
];
const LIST_MARKER_TAG: &str = "mf-listmarker";

static WORD_LIST_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<!\[if\s+!supportLists\]>(?P<m>.*?)<!\[endif\]>").unwrap());
static DOWNLEVEL_CONDITIONAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<!\[(?:if\s[^\]]*|endif)\]>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static MSO_LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)mso-list\s*:\s*(?P<id>l\d+)\s+level(?P<level>\d+)").unwrap());
static ORDERED_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\(（]?([0-9]+|[a-zA-Z]|[ivxlcdmIVXLCDM]+|[一二三四五六七八九十百]+)[\.\)）、．]").unwrap()
});
static STYLE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>(?P<css>.*?)</style>").unwrap());
static CSS_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/|<!--|-->").unwrap());
static CSS_AT_RULE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)@[a-z-]+[^{;]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}|@[^;{]+;").unwrap());
static CSS_RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?P<sel>[^{}]+)\{(?P<body>[^{}]*)\}").unwrap());
static CLASS_SELECTOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]*\.[a-z0-9_-]+$").unwrap());
static FONT_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<n>[0-9]*\.?[0-9]+)\s*(?P<u>pt|px|em|rem|%)?").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new("[ \t\r\n\x0C\u{00A0}]+").unwrap());
static MULTI_NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{2,}").unwrap());

type ClassStyles = HashMap<String, HashMap<String, String>>;
type N<'a> = NodeRef<'a, Node>;

/// 把 HTML 轉成 Markdown。`resolve_image` 把 `<img src>`（`file:///`、`data:`、網址…）
/// 對應成要寫進 Markdown 的路徑；回傳 `None` 保留原始 src。
pub fn html_to_markdown(html: &str, resolve_image: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let html = WORD_LIST_MARKER.replace_all(html, |caps: &regex::Captures| {
        let marker = decode_entities(&TAG.replace_all(&caps["m"], ""));
        let marker = WHITESPACE.replace_all(&marker, " ");
        format!("<{LIST_MARKER_TAG} data-m=\"{}\"></{LIST_MARKER_TAG}>", encode_attr(marker.trim()))
    });
    let html = DOWNLEVEL_CONDITIONAL.replace_all(&html, "");

    // 樣式表要從原始字串取：Word 在剪貼簿裡把 <style> 放在條件註解
    // <!--[if gte mso 10]> … <![endif]--> 裡，HTML 解析器根本看不到 <style> 元素。
    let class_styles = parse_style_sheets(&html);
    let doc = Html::parse_document(&html);
    let root = doc.tree.root();
    let body = find_element(root, "body").unwrap_or(root);

    let mut conv = Converter::new(&class_styles, resolve_image, false);
    conv.walk_blocks(body, TextStyle::default());
    conv.flush_inline();
    conv.promote_sized_paragraphs();
    let mut out = conv.serialize().trim_matches('\n').to_string();
    out.push('\n');
    out
}

// ============================== 區塊模型 ==============================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Paragraph,
    Heading,
    WordListItem,
    Raw,
}

#[derive(Debug)]
struct Block {
    kind: Kind,
    text: String,
    /// 標題層級，或 Word 清單深度（0 起算）
    level: usize,
    ordered: bool,
    /// Word 清單 id（mso-list 的 l0、l1…）；id 改變代表另一個清單，重新編號
    list_id: String,
    /// 內容是 ul／ol 轉出來的 Markdown 清單
    is_list: bool,
    /// 段落的主要字級（pt）
    dominant_size: Option<f64>,
}

impl Block {
    fn new(kind: Kind, text: String) -> Self {
        Block { kind, text, level: 0, ordered: false, list_id: String::new(), is_list: false, dominant_size: None }
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct TextStyle {
    size_pt: Option<f64>,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    in_pre: bool,
    in_heading: bool,
}

struct Converter<'s, 'r> {
    class_styles: &'s ClassStyles,
    resolve_image: &'r mut dyn FnMut(&str) -> Option<String>,
    nested: bool,
    blocks: Vec<Block>,
    inline: String,
    inline_sizes: HashMap<u64, usize>,
    inline_has_content: bool,
}

/// 字級以 0.5pt 為單位存成整數鍵（f64 不能當 HashMap 鍵）
fn size_key(pt: f64) -> u64 {
    (pt * 2.0).round() as u64
}
fn key_size(key: u64) -> f64 {
    key as f64 / 2.0
}

impl<'s, 'r> Converter<'s, 'r> {
    fn new(class_styles: &'s ClassStyles, resolve_image: &'r mut dyn FnMut(&str) -> Option<String>, nested: bool) -> Self {
        Converter {
            class_styles,
            resolve_image,
            nested,
            blocks: Vec::new(),
            inline: String::new(),
            inline_sizes: HashMap::new(),
            inline_has_content: false,
        }
    }

    // ---------- 走訪 ----------

    fn walk_blocks(&mut self, parent: N, style: TextStyle) {
        for node in parent.children() {
            match node.value() {
                Node::Comment(_) => continue,
                Node::Text(_) => {
                    self.append_inline(node, style);
                    continue;
                }
                Node::Element(_) => {}
                _ => continue,
            }
            let tag = tag_name(node);
            if !BLOCK_TAGS.contains(&tag.as_str()) || tag == LIST_MARKER_TAG {
                // 包著整段區塊的行內元素其實是容器——Google 文件把全部內容包在
                // <b style="font-weight:normal" id="docs-internal-guid-…"> 裡，當成行內處理會把標題、清單壓扁
                if !SKIP_TAGS.contains(&tag.as_str()) && has_block_descendant(node) {
                    let child_style = self.resolve_style(node, style);
                    self.walk_blocks(node, child_style);
                } else {
                    self.append_inline(node, style);
                }
                continue;
            }

            let child_style = self.resolve_style(node, style);
            match tag.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    self.flush_inline();
                    let heading_style = TextStyle { bold: true, in_heading: true, ..child_style };
                    let heading = collapse_lines(&self.render_inline_children(node, heading_style));
                    self.inline_sizes.clear(); // 標題不計入內文字級
                    if !heading.is_empty() {
                        let mut b = Block::new(Kind::Heading, heading);
                        b.level = (tag.as_bytes()[1] - b'0') as usize;
                        self.blocks.push(b);
                    }
                }
                "p" | "dt" | "dd" | "figcaption" | "summary" => {
                    self.flush_inline();
                    if self.try_word_list_item(node, child_style) {
                        continue;
                    }
                    self.walk_blocks(node, child_style);
                    self.flush_inline();
                }
                "ul" | "ol" => {
                    self.flush_inline();
                    let list = self.render_list(node, child_style);
                    self.add_raw(list, true);
                }
                "li" => {
                    // 清單外的零散 <li>（瀏覽器中只選了部分清單）
                    self.flush_inline();
                    let body = self.convert_nested(node, child_style);
                    self.add_raw(prefix_lines(&body, "- ", "  ", None), false);
                }
                "table" => {
                    self.flush_inline();
                    let t = self.render_table(node, child_style);
                    self.add_raw(t, false);
                }
                "pre" => {
                    self.flush_inline();
                    self.add_raw(render_pre(node), false);
                }
                "blockquote" => {
                    self.flush_inline();
                    let body = self.convert_nested(node, child_style);
                    self.add_raw(prefix_lines(&body, "> ", "> ", Some(">")), false);
                }
                "hr" => {
                    self.flush_inline();
                    self.add_raw("---".to_string(), false);
                }
                _ => {
                    // div、section、body、表格外的 tr…：透明容器
                    self.flush_inline();
                    self.walk_blocks(node, child_style);
                    self.flush_inline();
                }
            }
        }
    }

    /// Word 的清單項目是一般段落加上 mso-list 樣式，項目符號以文字寫出
    /// （前處理已換成 `<mf-listmarker data-m="1.">`）。
    fn try_word_list_item(&mut self, p: N, style: TextStyle) -> bool {
        let style_attr = attr(p, "style").unwrap_or_default();
        let level_match = MSO_LIST.captures(&style_attr);
        let marker_node = p.descendants().find(|d| d.value().is_element() && tag_name(*d) == LIST_MARKER_TAG);
        if level_match.is_none() && marker_node.is_none() {
            return false;
        }
        let marker = marker_node.and_then(|m| attr(m, "data-m")).unwrap_or_default();
        let level = level_match
            .as_ref()
            .and_then(|c| c["level"].parse::<usize>().ok())
            .map_or(0, |l| l.saturating_sub(1));
        let text = collapse_lines(&self.render_inline_children(p, style));
        self.inline_sizes.clear();

        let mut b = Block::new(Kind::WordListItem, text.trim().to_string());
        b.level = level;
        b.ordered = ORDERED_MARKER.is_match(&marker);
        b.list_id = level_match.map(|c| c["id"].to_string()).unwrap_or_default();
        self.blocks.push(b);
        true
    }

    fn append_inline(&mut self, node: N, style: TextStyle) {
        let rendered = self.render_inline(node, style);
        if !rendered.trim().is_empty() {
            self.inline_has_content = true;
        }
        self.inline.push_str(&rendered);
    }

    fn flush_inline(&mut self) {
        let text = std::mem::take(&mut self.inline);
        let sizes = std::mem::take(&mut self.inline_sizes);
        let had_content = std::mem::replace(&mut self.inline_has_content, false);
        if !had_content {
            return;
        }
        let text = tidy_paragraph(&text);
        if text.is_empty() {
            return;
        }
        let dominant = sizes.iter().max_by_key(|(k, v)| (**v, **k)).map(|(k, _)| key_size(*k));
        let mut b = Block::new(Kind::Paragraph, text);
        b.dominant_size = dominant;
        self.blocks.push(b);
    }

    fn add_raw(&mut self, text: String, is_list: bool) {
        let text = text.trim_matches('\n').to_string();
        if !text.trim().is_empty() {
            let mut b = Block::new(Kind::Raw, text);
            b.is_list = is_list;
            self.blocks.push(b);
        }
    }

    /// 清單項目、引用、表格儲存格的內容單獨轉換，不做字級標題判斷（只對最外層有意義）。
    fn convert_nested(&mut self, node: N, style: TextStyle) -> String {
        let mut nested = Converter::new(self.class_styles, &mut *self.resolve_image, true);
        nested.walk_blocks(node, style);
        nested.flush_inline();
        nested.serialize().trim_matches('\n').to_string()
    }

    // ---------- 行內 ----------

    fn render_inline_children(&mut self, node: N, style: TextStyle) -> String {
        let mut out = String::new();
        for child in node.children() {
            out.push_str(&self.render_inline(child, style));
        }
        out
    }

    fn render_inline(&mut self, node: N, parent: TextStyle) -> String {
        match node.value() {
            Node::Text(t) => {
                let raw: &str = t;
                if parent.in_pre {
                    return raw.to_string();
                }
                // Word 會在原始碼中硬換行；中文字之間的換行要直接拿掉，不能變成空格
                let text = WHITESPACE.replace_all(&remove_cjk_line_wraps(raw), " ").into_owned();
                let visible = text.chars().filter(|c| !c.is_whitespace()).count();
                if visible > 0
                    && let Some(size) = parent.size_pt {
                        *self.inline_sizes.entry(size_key(size)).or_default() += visible;
                    }
                return if parent.code { text } else { escape_inline(&text) };
            }
            Node::Element(_) => {}
            _ => return String::new(),
        }

        let tag = tag_name(node);
        if SKIP_TAGS.contains(&tag.as_str()) || tag == LIST_MARKER_TAG {
            return String::new();
        }
        if tag.contains(':') && tag != "o:p" {
            return String::new(); // VML／Office 命名空間的圖形
        }

        let style = self.resolve_style(node, parent);
        match tag.as_str() {
            "br" => return if parent.in_heading { " ".into() } else { "\n".into() },
            "img" => return self.render_image(node),
            "table" => {
                // 行內環境中的表格（例如 <span> 裡的 <td>）：攤平成文字
                let text = WHITESPACE.replace_all(&text_content(node), " ").trim().to_string();
                return format!(" {text} ");
            }
            _ => {}
        }

        if BLOCK_TAGS.contains(&tag.as_str()) {
            // 行內流程中的區塊（<a> 裡的 <p>、<td> 裡的 <div>…）：保留為換行
            let inner = self.render_inline_children(node, style);
            let inner = inner.trim();
            return if inner.is_empty() { String::new() } else { format!("\n{inner}\n") };
        }

        if style.code && !parent.code {
            let code = WHITESPACE.replace_all(&text_content(node), " ").into_owned();
            if code.trim().is_empty() {
                return code;
            }
            return if code.contains('`') { format!("`` {} ``", code.trim()) } else { format!("`{}`", code.trim()) };
        }

        let mut content = self.render_inline_children(node, style);

        if tag == "a" {
            let href = attr(node, "href").unwrap_or_default();
            let href = href.trim();
            if !href.is_empty()
                && !href.starts_with('#')
                && !href.to_ascii_lowercase().starts_with("javascript:")
                && !content.trim().is_empty()
            {
                let lead = if content.starts_with(' ') { " " } else { "" };
                let trail = if content.ends_with(' ') { " " } else { "" };
                return format!("{lead}[{}]({}){trail}", content.trim(), encode_url(href));
            }
            return content;
        }

        if style.strike && !parent.strike {
            content = wrap(&content, "~~");
        }
        if style.italic && !parent.italic {
            content = wrap(&content, "*");
        }
        if style.bold && !parent.bold {
            content = wrap(&content, "**");
        }
        if (tag == "sup" || tag == "sub") && !content.trim().is_empty() {
            content = format!("<{tag}>{}</{tag}>", content.trim());
        }
        content
    }

    fn render_image(&mut self, img: N) -> String {
        let src = attr(img, "src").unwrap_or_default();
        let src = src.trim();
        if src.is_empty() {
            return String::new();
        }
        let alt = attr(img, "alt").unwrap_or_default();
        let alt = WHITESPACE.replace_all(&alt, " ").trim().replace('[', "\\[").replace(']', "\\]");
        let path = (self.resolve_image)(src).unwrap_or_else(|| src.to_string());
        format!("![{alt}]({})", encode_url(&path))
    }

    // ---------- 清單／表格 ----------

    fn render_list(&mut self, list: N, style: TextStyle) -> String {
        let ordered = tag_name(list) == "ol";
        let mut number: i64 = attr(list, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1);
        let mut items: Vec<String> = Vec::new();

        for child in list.children() {
            if !child.value().is_element() {
                continue;
            }
            let child_tag = tag_name(child);
            if child_tag == "li" {
                let child_style = self.resolve_style(child, style);
                let body = self.convert_nested(child, child_style);
                let marker = if ordered { format!("{number}. ") } else { "- ".to_string() };
                number += 1;
                items.push(prefix_lines(&body, &marker, &" ".repeat(marker.chars().count()), None));
            } else if child_tag == "ul" || child_tag == "ol" {
                // 不規範的巢狀（<ul><li>a</li><ul>…</ul></ul>）：接到上一個項目
                let nested = self.render_list(child, style);
                if let Some(last) = items.last_mut() {
                    let indent = if ordered { " ".repeat((number - 1).to_string().len() + 2) } else { "  ".to_string() };
                    last.push('\n');
                    last.push_str(&prefix_lines(&nested, &indent, &indent, None));
                }
            }
        }
        items.join("\n")
    }

    fn render_table(&mut self, table_node: N, style: TextStyle) -> String {
        let table_id = table_node.id();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for tr in table_node.descendants() {
            if !tr.value().is_element() || tag_name(tr) != "tr" {
                continue;
            }
            // 只取這個表格自己的列，不含儲存格內巢狀表格的列
            let owner = tr.ancestors().find(|a| a.value().is_element() && tag_name(*a) == "table");
            if owner.map(|o| o.id()) != Some(table_id) {
                continue;
            }
            let mut cells = Vec::new();
            for cell in tr.children() {
                if !cell.value().is_element() {
                    continue;
                }
                let t = tag_name(cell);
                if t != "td" && t != "th" {
                    continue;
                }
                let cell_style = self.resolve_style(cell, style);
                // table::build 會跳脫 | 並把剩下的換行轉成 <br>
                let text = self.convert_nested(cell, cell_style);
                cells.push(MULTI_NEWLINE.replace_all(&text, "\n").into_owned());
                let span = attr(cell, "colspan").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 50);
                cells.extend(std::iter::repeat_n(String::new(), span - 1));
            }
            rows.push(cells);
        }
        table::build(&rows).unwrap_or_default()
    }

    // ---------- 字級標題 ----------

    fn promote_sized_paragraphs(&mut self) {
        // 沒有明確字級的段落（LibreOffice 內文用的是樣式預設值）也算進來，記在 -1；
        // 若它們佔多數，內文就視為 DEFAULT_BODY_PT。
        let mut histogram: HashMap<i64, usize> = HashMap::new();
        for b in self.blocks.iter().filter(|b| b.kind == Kind::Paragraph) {
            let key = b.dominant_size.map_or(-1, |s| size_key(s) as i64);
            *histogram.entry(key).or_default() += b.text.chars().count();
        }
        if !histogram.keys().any(|k| *k > 0) {
            return;
        }
        let body = if histogram.len() == 1 {
            DEFAULT_BODY_PT
        } else {
            let (key, _) = histogram.iter().max_by_key(|(k, v)| (**v, **k)).unwrap();
            if *key <= 0 { DEFAULT_BODY_PT } else { key_size(*key as u64) }
        };

        for b in &mut self.blocks {
            if b.kind != Kind::Paragraph {
                continue;
            }
            let Some(size) = b.dominant_size else { continue };
            if b.text.chars().count() > 120 || b.text.contains('\n') || b.text.starts_with("![") {
                continue;
            }
            let ratio = size / body;
            let level = if ratio >= 1.75 {
                1
            } else if ratio >= 1.45 {
                2
            } else if ratio >= 1.25 {
                3
            } else if ratio >= 1.12 {
                4
            } else {
                0
            };
            if level == 0 {
                continue;
            }
            b.kind = Kind::Heading;
            b.level = level;
            b.text = b.text.replace("**", "").trim().to_string();
        }
    }

    // ---------- 輸出 ----------

    fn serialize(&self) -> String {
        let mut out = String::new();
        let mut counters = [0usize; 10];
        let mut indents: [String; 10] = Default::default();
        let mut previous: Option<&Block> = None;

        for b in &self.blocks {
            // 同一個 Word 清單的項目之間不空行；清單項目內的子清單緊接著項目文字（空行會讓清單變成 loose）
            let tight = previous.is_some_and(|p| {
                (p.kind == Kind::WordListItem && b.kind == Kind::WordListItem && p.list_id == b.list_id)
                    || (self.nested && b.is_list && p.kind == Kind::Paragraph)
            });
            if !out.is_empty() {
                out.push_str(if tight { "\n" } else { "\n\n" });
            }

            match b.kind {
                Kind::Heading => {
                    out.push_str(&"#".repeat(b.level.clamp(1, 6)));
                    out.push(' ');
                    out.push_str(&b.text);
                }
                Kind::WordListItem => {
                    let level = b.level.min(counters.len() - 1);
                    if previous.is_none_or(|p| p.kind != Kind::WordListItem || p.list_id != b.list_id) {
                        counters = [0; 10];
                    }
                    for c in counters.iter_mut().skip(level + 1) {
                        *c = 0;
                    }
                    counters[level] += 1;
                    let indent: String =
                        (0..level).map(|i| if indents[i].is_empty() { "  ".to_string() } else { indents[i].clone() }).collect();
                    let marker = if b.ordered { format!("{}. ", counters[level]) } else { "- ".to_string() };
                    indents[level] = " ".repeat(marker.len());
                    out.push_str(&indent);
                    out.push_str(&marker);
                    out.push_str(&b.text);
                }
                _ => out.push_str(&b.text),
            }
            previous = Some(b);
        }
        out
    }

    // ---------- CSS ----------

    fn resolve_style(&self, node: N, parent: TextStyle) -> TextStyle {
        let mut style = parent;
        let tag = tag_name(node);
        match tag.as_str() {
            "b" | "strong" | "th" => style.bold = true,
            "i" | "em" | "cite" | "var" => style.italic = true,
            "s" | "strike" | "del" => style.strike = true,
            "code" | "kbd" | "samp" | "tt" => style.code = true,
            "pre" => style.in_pre = true,
            "font" => {
                if let Some(size) = attr(node, "size") {
                    let size = size.trim();
                    if let Ok(mut n) = size.parse::<i32>() {
                        if size.starts_with('+') || size.starts_with('-') {
                            n += 3;
                        }
                        const MAP: [f64; 8] = [7.5, 7.5, 10.0, 12.0, 13.5, 18.0, 24.0, 36.0];
                        style.size_pt = Some(MAP[n.clamp(1, 7) as usize]);
                    }
                }
            }
            _ => {}
        }

        // 先套 <style> 的 class 規則，最後套 style 屬性（優先）
        if let Some(classes) = attr(node, "class") {
            for cls in classes.split_whitespace() {
                let cls = cls.to_ascii_lowercase();
                if let Some(rule) = self.class_styles.get(&format!("{tag}.{cls}")) {
                    style = apply_declarations(style, parent, rule);
                } else if let Some(rule) = self.class_styles.get(&format!(".{cls}")) {
                    style = apply_declarations(style, parent, rule);
                }
            }
        }
        if let Some(inline) = attr(node, "style") {
            style = apply_declarations(style, parent, &parse_declarations(&inline));
        }
        style
    }
}

fn apply_declarations(mut style: TextStyle, parent: TextStyle, decls: &HashMap<String, String>) -> TextStyle {
    if let Some(pt) = decls.get("font-size").and_then(|v| parse_font_size(v, parent.size_pt)) {
        style.size_pt = Some(pt);
    }
    if let Some(weight) = decls.get("font-weight") {
        let w = weight.trim().to_ascii_lowercase();
        if w == "bold" || w == "bolder" {
            style.bold = true;
        } else if let Ok(n) = w.parse::<u32>() {
            style.bold = n >= 600;
        } else if w == "normal" || w == "lighter" {
            style.bold = false;
        }
    }
    if let Some(fs) = decls.get("font-style") {
        let fs = fs.trim().to_ascii_lowercase();
        if fs == "italic" || fs == "oblique" {
            style.italic = true;
        } else if fs == "normal" {
            style.italic = false;
        }
    }
    let decoration = format!(
        "{} {}",
        decls.get("text-decoration").map(String::as_str).unwrap_or(""),
        decls.get("text-decoration-line").map(String::as_str).unwrap_or("")
    );
    if decoration.to_ascii_lowercase().contains("line-through") {
        style.strike = true;
    }
    style
}

/// pt、px（×0.75）、em／rem／%（相對父層），四捨五入到 0.5pt。
fn parse_font_size(value: &str, parent_pt: Option<f64>) -> Option<f64> {
    let value = value.trim().to_ascii_lowercase();
    let caps = FONT_SIZE.captures(&value)?;
    let n: f64 = caps["n"].parse().ok()?;
    let base = parent_pt.unwrap_or(DEFAULT_BODY_PT);
    let pt = match caps.name("u").map_or("", |m| m.as_str()) {
        "pt" | "" => n,
        "px" => n * 0.75,
        "em" | "rem" => base * n,
        "%" => base * n / 100.0,
        _ => return None,
    };
    Some((pt * 2.0).round() / 2.0)
}

fn parse_declarations(css: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in css.split(';') {
        if let Some((k, v)) = part.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            if !k.is_empty() {
                map.insert(k, v.trim().trim_matches(['"', '\'']).to_string());
            }
        }
    }
    map
}

/// `<style>` 中的 class 規則，鍵為 "tag.class" 或 ".class"（Word 的 MsoTitle、MsoNormal 字級都在這裡）。
fn parse_style_sheets(html: &str) -> ClassStyles {
    let mut rules: ClassStyles = HashMap::new();
    for block in STYLE_BLOCK.captures_iter(html) {
        let css = CSS_COMMENT.replace_all(&block["css"], "");
        let css = CSS_AT_RULE.replace_all(&css, "");
        for rule in CSS_RULE.captures_iter(&css) {
            let decls = parse_declarations(&rule["body"]);
            for sel in rule["sel"].split(',') {
                let sel = sel.trim().to_ascii_lowercase();
                if !CLASS_SELECTOR.is_match(&sel) {
                    continue;
                }
                rules.entry(sel).or_default().extend(decls.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
    }
    rules
}

// ============================== 小工具 ==============================

fn tag_name(node: N) -> String {
    match node.value() {
        Node::Element(e) => e.name().to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn attr(node: N, name: &str) -> Option<String> {
    match node.value() {
        Node::Element(e) => e.attr(name).map(str::to_string),
        _ => None,
    }
}

fn find_element<'a>(root: N<'a>, name: &str) -> Option<N<'a>> {
    root.descendants().find(|n| n.value().is_element() && tag_name(*n) == name)
}

fn has_block_descendant(node: N) -> bool {
    node.descendants().skip(1).any(|d| d.value().is_element() && BLOCK_TAGS.contains(&tag_name(d).as_str()))
}

fn text_content(node: N) -> String {
    node.descendants()
        .filter_map(|d| match d.value() {
            Node::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}

fn render_pre(pre: N) -> String {
    fn collect(node: N, out: &mut String) {
        for c in node.children() {
            match c.value() {
                Node::Text(t) => out.push_str(t),
                Node::Element(_) if tag_name(c) == "br" => out.push('\n'),
                Node::Element(_) => collect(c, out),
                _ => {}
            }
        }
    }
    let mut code = String::new();
    collect(pre, &mut code);
    let code = code.replace("\r\n", "\n");
    let code = code.trim_matches('\n');

    let code_el = pre.descendants().find(|d| d.value().is_element() && tag_name(*d) == "code");
    let classes = format!(
        "{} {}",
        code_el.and_then(|c| attr(c, "class")).unwrap_or_default(),
        attr(pre, "class").unwrap_or_default()
    );
    let language = classes
        .split_whitespace()
        .find_map(|c| c.strip_prefix("language-").or_else(|| c.strip_prefix("lang-")))
        .unwrap_or("");
    let fence = if code.contains("```") { "~~~~" } else { "```" };
    format!("{fence}{language}\n{code}\n{fence}")
}

fn is_cjk(c: char) -> bool {
    ('\u{2E80}'..='\u{FFEF}').contains(&c)
}

/// 兩個 CJK 字元之間只有空白與換行（至少一個換行）時，整段刪除。
fn remove_cjk_line_wraps(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, ' ' | '\r' | '\n') && i > 0 && is_cjk(chars[i - 1]) {
            let mut j = i;
            let mut has_newline = false;
            while j < chars.len() && matches!(chars[j], ' ' | '\r' | '\n') {
                has_newline |= chars[j] == '\n';
                j += 1;
            }
            if has_newline && j < chars.len() && is_cjk(chars[j]) {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn tidy_paragraph(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_matches(|c: char| c == '\r' || c.is_whitespace())).collect();
    let start = lines.iter().position(|l| !l.is_empty());
    let end = lines.iter().rposition(|l| !l.is_empty());
    let (Some(start), Some(end)) = (start, end) else { return String::new() };

    let mut result: Vec<&str> = Vec::new();
    for line in &lines[start..=end] {
        if line.is_empty() && result.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        result.push(line);
    }
    // Word 常把一段粗體拆成多個 run：**a****b** → **ab**
    let joined = result.join("\n").replace("****", "").replace("~~~~", "");
    joined.split('\n').map(escape_line_start).collect::<Vec<_>>().join("\n")
}

/// 行首會被 Markdown 誤認的符號加上反斜線：`#`、`>`、`- `、`+ `、`1. `、整行的 `===`／`---`。
fn escape_line_start(line: &str) -> String {
    let trimmed = line.trim_start_matches(' ');
    let ws = &line[..line.len() - trimmed.len()];
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let rest = &trimmed[digits.len()..];
        let mut it = rest.chars();
        if let (Some(d @ ('.' | ')')), Some(next)) = (it.next(), it.next())
            && next.is_whitespace() {
                return format!("{ws}{digits}\\{d}{}", &rest[1..]);
            }
        return line.to_string();
    }
    let mut chars = trimmed.chars();
    let first = chars.next();
    let second = chars.next();
    let only = |c: char| !trimmed.is_empty() && trimmed.trim_end().chars().all(|x| x == c);
    match first {
        Some('#') | Some('>') => format!("{ws}\\{trimmed}"),
        Some('-') | Some('+') if second.is_some_and(char::is_whitespace) => format!("{ws}\\{trimmed}"),
        Some('=') if only('=') => format!("{ws}\\{trimmed}"),
        Some('-') if only('-') => format!("{ws}\\{trimmed}"),
        _ => line.to_string(),
    }
}

fn collapse_lines(text: &str) -> String {
    tidy_paragraph(text).replace('\n', " ").trim().to_string()
}

fn prefix_lines(text: &str, first: &str, rest: &str, blank_prefix: Option<&str>) -> String {
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.is_empty() && i > 0 {
            out.push_str(blank_prefix.unwrap_or(""));
            continue;
        }
        out.push_str(if i == 0 { first } else { rest });
        out.push_str(line);
    }
    out
}

/// 強調符號不可跨硬換行，每一行分別包覆；前後空白留在符號外。
fn wrap(content: &str, marker: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return content.to_string();
    }
    let lead = &content[..content.len() - content.trim_start().len()];
    let trail = &content[content.trim_end().len()..];
    let lines: Vec<String> = trimmed
        .split('\n')
        .map(|l| if l.trim().is_empty() { l.to_string() } else { format!("{marker}{}{marker}", l.trim()) })
        .collect();
    format!("{lead}{}{trail}", lines.join("\n"))
}

fn escape_inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        match c {
            '*' | '`' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '\\' if next.is_some_and(|n| n.is_ascii_punctuation()) => out.push_str("\\\\"),
            '_' if !prev.is_some_and(char::is_alphanumeric) || !next.is_some_and(char::is_alphanumeric) => {
                out.push_str("\\_")
            }
            '<' if next.is_some_and(|n| n.is_alphabetic() || n == '/' || n == '!') => out.push_str("&lt;"),
            _ => out.push(c),
        }
    }
    out
}

fn encode_url(url: &str) -> String {
    let url = url.trim();
    if url.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
}

fn encode_attr(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 只需辨識清單符號，處理常見實體即可。
fn decode_entities(text: &str) -> String {
    static NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|\d+);").unwrap());
    let text = NUMERIC.replace_all(text, |c: &regex::Captures| {
        let v = &c[1];
        let n = if let Some(hex) = v.strip_prefix('x') { u32::from_str_radix(hex, 16).ok() } else { v.parse().ok() };
        n.and_then(char::from_u32).map(String::from).unwrap_or_default()
    });
    text.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(html: &str) -> String {
        html_to_markdown(html, &mut |src| Some(format!("[img:{}]", src.rsplit('/').next().unwrap_or(src))))
    }

    #[test]
    fn google_docs_wrapper_title_heading_list_link() {
        let html = r#"<meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-abc"><p dir="ltr"><span style="font-size:26pt;font-weight:400;">文件標題</span></p><h2 dir="ltr"><span style="font-size:16pt;font-weight:400;">第二層標題</span></h2><p dir="ltr"><span style="font-size:11pt;font-weight:400;">一般文字 </span><span style="font-size:11pt;font-weight:700;">粗體</span><span style="font-size:11pt;font-weight:400;"> 與 </span><a href="https://example.com/a b"><span style="font-size:11pt;">連結</span></a></p><ul><li dir="ltr"><p dir="ltr"><span style="font-size:11pt;">項目一</span></p></li><li dir="ltr"><p dir="ltr"><span style="font-size:11pt;">項目二</span></p><ul><li><p><span style="font-size:11pt;">子項目</span></p></li></ul></li></ul></b>"#;
        assert_eq!(
            convert(html),
            "# 文件標題\n\n## 第二層標題\n\n一般文字 **粗體** 與 [連結](<https://example.com/a b>)\n\n- 項目一\n- 項目二\n  - 子項目\n"
        );
    }

    #[test]
    fn word_nested_list_from_supportlists_markers() {
        let html = "<html xmlns:w=\"urn:schemas-microsoft-com:office:word\"><body>\
            <p class=MsoListParagraph style='margin-left:24pt;mso-list:l0 level1 lfo1'><![if !supportLists]><span>1.<span>&nbsp;&nbsp; </span></span><![endif]>第一步</p>\
            <p class=MsoListParagraph style='margin-left:48pt;mso-list:l0 level2 lfo1'><![if !supportLists]><span>●<span>&nbsp; </span></span><![endif]>子步驟 A</p>\
            <p class=MsoListParagraph style='margin-left:48pt;mso-list:l0 level2 lfo1'><![if !supportLists]><span>●<span>&nbsp; </span></span><![endif]>子步驟 B</p>\
            <p class=MsoListParagraph style='margin-left:24pt;mso-list:l0 level1 lfo1'><![if !supportLists]><span>2.<span>&nbsp;&nbsp; </span></span><![endif]>第二步</p>\
            <p class=MsoNormal>一般段落</p></body></html>";
        assert_eq!(convert(html), "1. 第一步\n   - 子步驟 A\n   - 子步驟 B\n2. 第二步\n\n一般段落\n");
    }

    #[test]
    fn different_word_lists_restart_numbering() {
        let html = "<p style='mso-list:l1 level1 lfo1'><![if !supportLists]>·<![endif]>a</p>\
                    <p style='mso-list:l0 level1 lfo2'><![if !supportLists]>1.<![endif]>b</p>";
        assert_eq!(convert(html), "- a\n\n1. b\n");
    }

    #[test]
    fn table_with_pipes_and_numbers() {
        let html = "<table><tr><th>品項</th><th>數量</th></tr><tr><td>香蕉|芭樂</td><td>200</td></tr></table>";
        assert_eq!(convert(html), "| 品項       | 數量 |\n| ---------- | ---: |\n| 香蕉\\|芭樂 |  200 |\n");
    }

    #[test]
    fn escaping_and_cjk_line_wraps() {
        let html = "<p>2*3 與 snake_case 與 _x_ 這是一段很長的文字在中\r\n文字之間被換行</p><p># 不是標題</p><p>1. 不是清單</p>";
        assert_eq!(
            convert(html),
            "2\\*3 與 snake_case 與 \\_x\\_ 這是一段很長的文字在中文字之間被換行\n\n\\# 不是標題\n\n1\\. 不是清單\n"
        );
    }

    #[test]
    fn images_go_through_resolver_and_pre_keeps_language() {
        let html = "<p><img src=\"file:///C:/tmp/clip_image001.png\" alt=\"圖\"></p><pre><code class=\"language-rust\">fn a() {}\n</code></pre>";
        assert_eq!(convert(html), "![圖]([img:clip_image001.png])\n\n```rust\nfn a() {}\n```\n");
    }

    #[test]
    fn adjacent_bold_runs_merge() {
        assert_eq!(convert("<p><b>Hel</b><b>lo</b> world</p>"), "**Hello** world\n");
    }

    #[test]
    fn line_start_escapes() {
        assert_eq!(escape_line_start("- a"), "\\- a");
        assert_eq!(escape_line_start("---"), "\\---");
        assert_eq!(escape_line_start("12) x"), "12\\) x");
        assert_eq!(escape_line_start("-a"), "-a");
        assert_eq!(escape_line_start("2026 年"), "2026 年");
    }
}
