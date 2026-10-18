//! 文件引用、但還不在文件資料夾裡的檔案：拖入的圖片與附件、貼上的截圖、Word 內容中的圖片。
//! 存檔時把它們複製到 .md 所在資料夾。
//!
//! 在那之前，每個檔案以 Markdown 中已使用的相對名稱（`photo.png`）登記，對應到目前實際的位置
//! （原始檔，或剪貼簿資料沒有檔案時存在本次工作階段的暫存資料夾）。預覽透過 [`DocumentAssets::resolve`]
//! 找檔，所以即使文件從未存檔，圖片也能立即顯示。

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

pub const IMAGE_EXTENSIONS: &[&str] =
    &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "tif", "tiff", "avif"];

static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"!?\[(?:[^\]\\]|\\.)*\]\(\s*(?:<(?P<angle>[^>\r\n]+)>|(?P<plain>[^\s)]+))").unwrap()
});
static HTML_SRC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<(?:img|source|video|audio)\b[^>]*?\ssrc\s*=\s*["'](?P<url>[^"']+)["']"#).unwrap()
});
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:").unwrap());

pub struct DocumentAssets {
    /// 小寫相對名稱 → (原樣的相對名稱, 目前實際位置)
    pending: HashMap<String, (String, PathBuf)>,
    staging_dir: PathBuf,
    document_dir: Option<PathBuf>,
    counter: u32,
}

impl DocumentAssets {
    /// `staging_root` 下會建立本工作階段專用的子資料夾（隨機名稱），結束時刪除。
    pub fn new(staging_root: &Path) -> Self {
        DocumentAssets {
            pending: HashMap::new(),
            staging_dir: staging_root.join(random_id()),
            document_dir: None,
            counter: 0,
        }
    }

    /// .md 所在資料夾；未存檔的新文件為 `None`。
    pub fn document_dir(&self) -> Option<&Path> {
        self.document_dir.as_deref()
    }

    /// 切換到另一份文件（新增／開啟）：清掉尚未存檔的登記。
    pub fn reset(&mut self, document_path: Option<&Path>) {
        self.pending.clear();
        self.document_dir = document_path.map(|p| absolute(p).parent().map(Path::to_path_buf).unwrap_or_default());
    }

    pub fn is_image(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    }

    /// 登記既有檔案，回傳 Markdown 中要使用的相對路徑。已在文件資料夾內的檔案直接引用原檔。
    pub fn add_file(&mut self, source: &Path, preferred_name: Option<&str>) -> io::Result<String> {
        let full = absolute(source);
        if let Some(doc_dir) = &self.document_dir
            && let Ok(rel) = full.strip_prefix(doc_dir)
                && !rel.as_os_str().is_empty() {
                    return Ok(to_markdown_path(rel));
                }

        let name = sanitize_file_name(
            preferred_name.map(str::to_string).unwrap_or_else(|| {
                full.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())
            }),
        );
        let (stem, ext) = split_name(&name);

        for i in 1.. {
            let candidate = if i == 1 { name.clone() } else { format!("{stem}-{i}{ext}") };
            let key = candidate.to_lowercase();
            if let Some((_, existing)) = self.pending.get(&key) {
                if same_path(existing, &full) || files_equal(existing, &full) {
                    return Ok(candidate);
                }
                continue;
            }
            if let Some(doc_dir) = &self.document_dir {
                let target = doc_dir.join(&candidate);
                if target.exists() {
                    if files_equal(&target, &full) {
                        return Ok(candidate); // 已經在那裡，不必複製
                    }
                    continue;
                }
            }
            self.pending.insert(key, (candidate.clone(), full));
            return Ok(candidate);
        }
        unreachable!()
    }

    /// 不會留存的檔案（Word 的 msohtmlclip 暫存圖）先複製到暫存區，再以易讀的名稱登記。
    pub fn add_transient_file(&mut self, source: &Path) -> io::Result<String> {
        let ext = source.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let staged = self.new_staging_path(&ext)?;
        fs::copy(source, &staged)?;
        let name = staged.file_name().map(|n| n.to_string_lossy().into_owned());
        self.add_file(&staged, name.as_deref())
    }

    /// 登記圖片位元組（剪貼簿 PNG、data: URI）。`extension` 例如 ".png"。
    pub fn add_image_bytes(&mut self, bytes: &[u8], extension: &str) -> io::Result<String> {
        let staged = self.new_staging_path(extension)?;
        fs::write(&staged, bytes)?;
        let name = staged.file_name().map(|n| n.to_string_lossy().into_owned());
        self.add_file(&staged, name.as_deref())
    }

    /// Markdown 中的相對路徑 → 目前存放該檔案的位置（先找未存檔的登記，再找文件資料夾）。
    pub fn resolve(&self, relative: &str) -> Option<PathBuf> {
        let rel = normalize_reference(relative);
        if rel.is_empty() {
            return None;
        }
        if let Some((_, path)) = self.pending.get(&rel.to_lowercase())
            && path.is_file() {
                return Some(path.clone());
            }
        let doc_dir = self.document_dir.as_ref()?;
        let candidate = doc_dir.join(&rel);
        // 不允許以 ../ 跑出文件資料夾之外
        if !is_within(doc_dir, &candidate) {
            return None;
        }
        candidate.is_file().then_some(candidate)
    }

    /// 把 Markdown 引用到的相對路徑檔案複製到 `document_path` 所在資料夾：未存檔的登記，
    /// 以及（另存到不同資料夾時）原本放在舊文件旁的檔案。同名但內容不同時改名為 `-2`、`-3`…
    /// 並改寫 Markdown 中的連結。回傳（可能改寫過的）Markdown 與無法複製的項目。
    pub fn commit_for_save(&mut self, markdown: &str, document_path: &Path) -> (String, Vec<String>) {
        let target_dir = absolute(document_path).parent().map(Path::to_path_buf).unwrap_or_default();
        let previous_dir = self.document_dir.clone();
        let mut failures = Vec::new();
        let mut renames: Vec<(String, String)> = Vec::new();

        for url in find_relative_references(markdown) {
            let rel = normalize_reference(&url);
            if rel.is_empty() {
                continue;
            }
            let key = rel.to_lowercase();
            let source = if let Some((_, path)) = self.pending.get(&key) {
                Some(path.clone())
            } else {
                previous_dir
                    .as_ref()
                    .filter(|prev| !same_path(prev, &target_dir))
                    .map(|prev| prev.join(&rel))
                    .filter(|p| p.is_file())
            };
            let Some(source) = source else { continue };
            if !source.is_file() {
                failures.push(rel);
                continue;
            }

            let mut target = target_dir.join(&rel);
            if same_path(&source, &target) {
                self.pending.remove(&key);
                continue;
            }
            if target.exists() && !files_equal(&source, &target) {
                let dir = target.parent().map(Path::to_path_buf).unwrap_or_else(|| target_dir.clone());
                let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (stem, ext) = split_name(&file_name);
                for i in 2.. {
                    target = dir.join(format!("{stem}-{i}{ext}"));
                    if !target.exists() || files_equal(&source, &target) {
                        break;
                    }
                }
                let new_rel = target.strip_prefix(&target_dir).map(to_markdown_path).unwrap_or(rel.clone());
                renames.push((url.clone(), new_rel));
            }
            if !target.exists() {
                let result = target.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::copy(&source, &target));
                if let Err(e) = result {
                    failures.push(format!("{rel}（{e}）"));
                    continue;
                }
            }
            self.pending.remove(&key);
        }

        let mut markdown = markdown.to_string();
        for (from, to) in renames {
            markdown = replace_reference(&markdown, &from, &to);
        }
        self.document_dir = Some(target_dir);
        (markdown, failures)
    }

    fn new_staging_path(&mut self, extension: &str) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.staging_dir)?;
        let ext = if extension.is_empty() { ".png".to_string() } else { extension.to_ascii_lowercase() };
        let stamp = local_timestamp();
        loop {
            self.counter += 1;
            let path = self.staging_dir.join(format!("image-{stamp}-{}{ext}", self.counter));
            if !path.exists() {
                return Ok(path);
            }
        }
    }
}

impl Drop for DocumentAssets {
    fn drop(&mut self) {
        // 盡力而為：在 %TEMP% 底下
        let _ = fs::remove_dir_all(&self.staging_dir);
    }
}

/// Markdown 用的路徑（正斜線）。
pub fn to_markdown_path(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// 連結目的地：含空白或括號的路徑會提早結束一般的連結目的地，改用 `<…>`。
pub fn link_destination(rel: &str) -> String {
    if rel.contains([' ', '(', ')']) { format!("<{rel}>") } else { rel.to_string() }
}

// ============================== 內部 ==============================

fn find_relative_references(markdown: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let md_urls = MARKDOWN_LINK
        .captures_iter(markdown)
        .filter_map(|c| c.name("angle").or_else(|| c.name("plain")).map(|m| m.as_str().trim().to_string()));
    let html_urls = HTML_SRC.captures_iter(markdown).map(|c| c["url"].trim().to_string());
    for url in md_urls.chain(html_urls) {
        if is_relative(&url) && !seen.contains(&url) {
            seen.push(url);
        }
    }
    seen
}

fn is_relative(url: &str) -> bool {
    !(url.is_empty() || url.starts_with(['#', '/', '\\']) || SCHEME.is_match(url))
}

fn normalize_reference(url: &str) -> String {
    let url = url.split(['#', '?']).next().unwrap_or("");
    let url = percent_decode(url).replace('\\', "/");
    url.strip_prefix("./").unwrap_or(&url).trim_start_matches('/').to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (hex(bytes.get(i + 1)), hex(bytes.get(i + 2))) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn hex(b: Option<&u8>) -> Option<u8> {
    match b? {
        c @ b'0'..=b'9' => Some(c - b'0'),
        c @ b'a'..=b'f' => Some(c - b'a' + 10),
        c @ b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// 只替換連結目的地位置上的 `from`（左邊是 `(`、`<`、引號，右邊是 `)`、`>`、引號或空白）。
fn replace_reference(markdown: &str, from: &str, to: &str) -> String {
    let to_url = to.replace(' ', "%20");
    let re = Regex::new(&format!(r#"([(<"']){}([)>"'\s])"#, regex::escape(from))).unwrap();
    re.replace_all(markdown, |c: &regex::Captures| format!("{}{}{}", &c[1], to_url, &c[2])).into_owned()
}

fn sanitize_file_name(name: String) -> String {
    let cleaned: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    if cleaned.trim().is_empty() { "file".to_string() } else { cleaned }
}

fn split_name(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// 不經過 canonicalize（那會產生 \\?\ 前綴、且要求檔案存在），只做絕對化與 . / .. 化簡。
fn absolute(path: &Path) -> PathBuf {
    let base = if path.is_absolute() { PathBuf::new() } else { std::env::current_dir().unwrap_or_default() };
    let mut out = PathBuf::new();
    for comp in base.join(path).components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_within(dir: &Path, path: &Path) -> bool {
    absolute(path).starts_with(absolute(dir))
}

fn same_path(a: &Path, b: &Path) -> bool {
    let a = absolute(a).to_string_lossy().to_lowercase();
    let b = absolute(b).to_string_lossy().to_lowercase();
    a.trim_end_matches(['\\', '/']) == b.trim_end_matches(['\\', '/'])
}

/// 大小不同即不同；64 MB 以下逐位元組比較，更大的檔案比較修改時間。
fn files_equal(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    if !ma.is_file() || !mb.is_file() || ma.len() != mb.len() {
        return false;
    }
    if ma.len() > 64 * 1024 * 1024 {
        return ma.modified().ok() == mb.modified().ok();
    }
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    let mut ba = vec![0u8; 81920];
    let mut bb = vec![0u8; 81920];
    loop {
        let (Ok(na), Ok(nb)) = (read_full(&mut fa, &mut ba), read_full(&mut fb, &mut bb)) else { return false };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

fn read_full(f: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = f.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    format!("{:012x}", h.finish() & 0xFFFF_FFFF_FFFF)
}

/// 本地時間 yyyyMMdd-HHmmss（暫存圖片檔名用）。
fn local_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("mf-attach-test-{tag}-{}", random_id()));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn untitled_then_save_with_conflict_then_save_as() {
        let root = TempDir::new("flow");
        let src = root.0.join("src");
        let doc_a = root.0.join("docA");
        let doc_b = root.0.join("docB");
        for d in [&src, &doc_a, &doc_b] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(src.join("photo.png"), "PHOTO-1").unwrap();
        fs::write(src.join("my file.pdf"), "PDF").unwrap();
        fs::write(doc_a.join("photo.png"), "DIFFERENT").unwrap(); // 目的地有同名不同內容的檔案

        let mut assets = DocumentAssets::new(&root.0.join("staging"));
        assets.reset(None);
        let rel1 = assets.add_file(&src.join("photo.png"), None).unwrap();
        let rel2 = assets.add_file(&src.join("my file.pdf"), None).unwrap();
        assert_eq!((rel1.as_str(), rel2.as_str()), ("photo.png", "my file.pdf"));
        assert_eq!(assets.resolve("photo.png"), Some(absolute(&src.join("photo.png"))));

        let md = format!("![p]({})\n[f]({})\n", link_destination(&rel1), link_destination(&rel2));
        let (md2, failures) = assets.commit_for_save(&md, &doc_a.join("a.md"));
        assert!(failures.is_empty());
        assert_eq!(md2, "![p](photo-2.png)\n[f](<my file.pdf>)\n");
        assert_eq!(fs::read_to_string(doc_a.join("photo-2.png")).unwrap(), "PHOTO-1");
        assert_eq!(fs::read_to_string(doc_a.join("photo.png")).unwrap(), "DIFFERENT");

        // 另存新檔到別的資料夾：舊文件旁的檔案也要帶過去
        let (md3, failures) = assets.commit_for_save(&md2, &doc_b.join("b.md"));
        assert!(failures.is_empty());
        assert_eq!(md3, md2);
        assert_eq!(fs::read_to_string(doc_b.join("photo-2.png")).unwrap(), "PHOTO-1");
        assert!(doc_b.join("my file.pdf").is_file());

        // 已經在文件資料夾裡的檔案直接引用
        assert_eq!(assets.add_file(&doc_b.join("photo-2.png"), None).unwrap(), "photo-2.png");
    }

    #[test]
    fn identical_file_in_document_folder_is_reused() {
        let root = TempDir::new("reuse");
        let doc = root.0.join("doc");
        fs::create_dir_all(&doc).unwrap();
        fs::write(doc.join("a.png"), "SAME").unwrap();
        fs::write(root.0.join("a.png"), "SAME").unwrap();
        fs::write(root.0.join("b.png"), "OTHER").unwrap();
        let mut assets = DocumentAssets::new(&root.0.join("staging"));
        assets.reset(Some(&doc.join("x.md")));
        assert_eq!(assets.add_file(&root.0.join("a.png"), None).unwrap(), "a.png");
        assert!(assets.pending.is_empty(), "identical file should not be copied again");
        assert_eq!(assets.add_file(&root.0.join("b.png"), Some("a.png")).unwrap(), "a-2.png");
    }

    #[test]
    fn staged_clipboard_image_and_cleanup() {
        let root = TempDir::new("stage");
        let staging_root = root.0.join("staging");
        let staged_dir;
        {
            let mut assets = DocumentAssets::new(&staging_root);
            let rel = assets.add_image_bytes(b"\x89PNG", ".png").unwrap();
            assert!(rel.starts_with("image-") && rel.ends_with("-1.png"), "{rel}");
            let path = assets.resolve(&rel).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"\x89PNG");
            staged_dir = path.parent().unwrap().to_path_buf();
        }
        assert!(!staged_dir.exists(), "staging folder removed on drop");
    }

    #[test]
    fn references_and_resolution_rules() {
        let md = "![a](img/a%20b.png) [x](https://e.com/x.png) ![c](<c d.png>) <img src=\"e.png\"> [n](#top) ![abs](C:/x.png)";
        assert_eq!(find_relative_references(md), ["img/a%20b.png", "<c d.png>".trim_matches(['<', '>']), "e.png"]);
        assert_eq!(normalize_reference("./img/a%20b.png#frag"), "img/a b.png");

        let root = TempDir::new("resolve");
        let mut assets = DocumentAssets::new(&root.0.join("staging"));
        fs::write(root.0.join("secret.txt"), "x").unwrap();
        fs::create_dir_all(root.0.join("doc")).unwrap();
        assets.reset(Some(&root.0.join("doc").join("d.md")));
        assert!(assets.resolve("../secret.txt").is_none(), "must not escape the document folder");
    }
}
