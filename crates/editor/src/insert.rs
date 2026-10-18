//! 拖入、貼上、「插入」選單 → 要插入編輯區的 Markdown。

use std::path::{Path, PathBuf};

use base64::Engine;
use markforge_core::attachments::{DocumentAssets, link_destination};
use markforge_core::import::{clipboard as cfhtml, html_to_markdown, spreadsheet, table};
use serde::Serialize;

use crate::clipboard;

const MERMAID_EXTENSIONS: &[&str] = &["mmd", "mermaid"];
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// 要插入的內容；`block` 表示前後需要空行（表格、程式碼區塊、多段落）。
#[derive(Serialize, Debug, Clone)]
pub struct Snippet {
    pub text: String,
    pub block: bool,
}

pub fn is_markdown_file(path: &Path) -> bool {
    ext(path).is_some_and(|e| MARKDOWN_EXTENSIONS.contains(&e.as_str()))
}

fn ext(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
}

/// 多個檔案 → 一段 Markdown；讀取失敗的檔案列在錯誤清單中，其餘照常插入。
pub fn files(paths: &[PathBuf], assets: &mut DocumentAssets) -> (Option<Snippet>, Vec<String>) {
    let mut parts: Vec<Snippet> = Vec::new();
    let mut errors = Vec::new();
    for path in paths.iter().filter(|p| p.is_file()) {
        match file(path, assets) {
            Ok(s) => parts.push(s),
            Err(e) => errors.push(format!("{}：{e}", path.file_name().unwrap_or_default().to_string_lossy())),
        }
    }
    if parts.is_empty() {
        return (None, errors);
    }
    let block = parts.iter().any(|p| p.block);
    let text = parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(if block { "\n\n" } else { "\n" });
    (Some(Snippet { text, block }), errors)
}

fn file(path: &Path, assets: &mut DocumentAssets) -> Result<Snippet, String> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();

    if DocumentAssets::is_image(path) {
        let rel = assets.add_file(path, None).map_err(|e| e.to_string())?;
        return Ok(Snippet { text: format!("![{}]({})", escape_link_text(&stem), link_destination(&rel)), block: false });
    }

    if ext(path).is_some_and(|e| MERMAID_EXTENSIONS.contains(&e.as_str())) {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let source = spreadsheet::decode_text(&bytes, spreadsheet::system_ansi_encoding()).replace("\r\n", "\n");
        return Ok(Snippet { text: format!("```mermaid\n{}\n```", source.trim_matches('\n')), block: true });
    }

    if spreadsheet::is_supported(path) {
        let sheets: Vec<(String, String)> = spreadsheet::read(path)?
            .into_iter()
            .filter_map(|s| table::build(&s.rows).map(|t| (s.name, t)))
            .collect();
        return match sheets.len() {
            0 => Err("檔案中沒有任何資料。".into()),
            1 => Ok(Snippet { text: sheets[0].1.trim_end().to_string(), block: true }),
            _ => Ok(Snippet {
                text: sheets
                    .iter()
                    .map(|(name, t)| format!("**{}**\n\n{}", escape_link_text(name), t.trim_end()))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                block: true,
            }),
        };
    }

    // 其他檔案：附件連結，存檔時複製到文件資料夾
    let rel = assets.add_file(path, None).map_err(|e| e.to_string())?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    Ok(Snippet { text: format!("[{}]({})", escape_link_text(&name), link_destination(&rel)), block: false })
}

fn escape_link_text(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]").replace('*', "\\*")
}

/// 貼上的結果。
pub enum Paste {
    /// 轉換後的 Markdown
    Markdown(Snippet),
    /// 原樣的純文字
    Text(String),
    /// 插入檔案（檔案總管複製的檔案）；可能附帶錯誤
    Files(Option<Snippet>, Vec<String>),
    Nothing,
}

/// 依剪貼簿內容決定貼上什麼。順序很重要：Excel 與 Word 複製時都會**同時**放一張點陣圖，
/// 所以文字類格式要先於圖片判斷。
pub fn paste(assets: &mut DocumentAssets, plain_only: bool) -> Paste {
    let Some(snap) = clipboard::read(plain_only) else { return Paste::Nothing };
    if plain_only {
        return snap.text.map_or(Paste::Nothing, |t| Paste::Text(t.replace("\r\n", "\n")));
    }

    if !snap.files.is_empty() {
        let (snippet, errors) = files(&snap.files, assets);
        return Paste::Files(snippet, errors);
    }

    // 試算表儲存格：Tab 分隔的文字最忠實
    let html = snap.html.as_deref().map(cfhtml::extract);
    let from_spreadsheet = snap.spreadsheet || html.is_some_and(cfhtml::is_spreadsheet_html);
    if from_spreadsheet
        && let Some(text) = &snap.text
            && let Some(t) = table::build(&table::parse_delimited(text.trim_end_matches(['\r', '\n']), '\t')) {
                return Paste::Markdown(Snippet { text: t.trim_end().to_string(), block: true });
            }

    if let Some(html) = html
        && (cfhtml::is_from_document_app(html) || cfhtml::has_markdown_worthy_structure(html)) {
            let md = html_to_markdown(html, &mut |src| resolve_pasted_image(src, assets));
            let md = md.trim_matches('\n');
            if !md.trim().is_empty() {
                let block = md.contains('\n') || md.starts_with('#') || md.starts_with('|');
                return Paste::Markdown(Snippet { text: md.to_string(), block });
            }
        }

    if let Some(text) = snap.text {
        if table::looks_like_tsv(&text)
            && let Some(t) = table::build(&table::parse_delimited(text.trim_end_matches(['\r', '\n']), '\t')) {
                return Paste::Markdown(Snippet { text: t.trim_end().to_string(), block: true });
            }
        return Paste::Text(text.replace("\r\n", "\n"));
    }

    let image = snap.png.or_else(|| snap.dib.as_deref().and_then(clipboard::dib_to_png));
    if let Some(png) = image
        && let Ok(rel) = assets.add_image_bytes(&png, ".png") {
            return Paste::Markdown(Snippet { text: format!("![]({})", link_destination(&rel)), block: false });
        }
    Paste::Nothing
}

/// 貼上 HTML 中的圖片：Word 指向 msohtmlclip 暫存檔（剪貼簿一換就消失），Google 文件／
/// 瀏覽器可能是 data: URI——兩者都先存到暫存區，存檔時再放到文件旁。網址維持不變。
fn resolve_pasted_image(src: &str, assets: &mut DocumentAssets) -> Option<String> {
    if let Some(rest) = src.strip_prefix("file:///").or_else(|| src.strip_prefix("file://")) {
        let local = PathBuf::from(percent_decode(rest).replace('/', "\\"));
        return local.is_file().then(|| assets.add_transient_file(&local).ok()).flatten();
    }
    if let Some(rest) = src.strip_prefix("data:image/") {
        let (header, data) = rest.split_once(',')?;
        let (mime, encoding) = header.split_once(';')?;
        if !encoding.eq_ignore_ascii_case("base64") {
            return None;
        }
        let ext = match mime {
            "jpeg" => ".jpg".to_string(),
            "svg+xml" => ".svg".to_string(),
            m => format!(".{m}"),
        };
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
        return assets.add_image_bytes(&bytes, &ext).ok();
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
