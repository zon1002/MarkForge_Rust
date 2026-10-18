//! 拖入編輯區的試算表檔案：`.xlsx`／`.xlsm`／`.xls`／`.xlsb`／`.ods` 由 calamine 讀取，
//! `.csv`／`.tsv` 以分隔文字解析。儲存格以「看起來的樣子」輸出：日期 `yyyy-MM-dd`、
//! 數字不帶多餘小數位、布林 `TRUE`／`FALSE`。

use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use encoding_rs::Encoding;

use super::table;

/// 一個工作表的儲存格文字。
#[derive(Debug, Clone)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

const WORKBOOK_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "xlsb", "ods"];
const DELIMITED_EXTENSIONS: &[&str] = &["csv", "tsv"];

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn is_supported(path: &Path) -> bool {
    let ext = extension(path);
    WORKBOOK_EXTENSIONS.contains(&ext.as_str()) || DELIMITED_EXTENSIONS.contains(&ext.as_str())
}

/// 讀取所有工作表（CSV／TSV 只有一個）。
pub fn read(path: &Path) -> Result<Vec<Sheet>, String> {
    let ext = extension(path);
    if DELIMITED_EXTENSIONS.contains(&ext.as_str()) {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let text = decode_text(&bytes, system_ansi_encoding());
        let delimiter = if ext == "tsv" { '\t' } else { table::guess_delimiter(&text) };
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Sheet").to_string();
        return Ok(vec![Sheet { name, rows: table::parse_delimited(&text, delimiter) }]);
    }

    let mut workbook = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let mut sheets = Vec::new();
    for name in workbook.sheet_names() {
        let range = workbook.worksheet_range(&name).map_err(|e| e.to_string())?;
        let rows = range.rows().map(|r| r.iter().map(format_cell).collect()).collect();
        sheets.push(Sheet { name, rows });
    }
    Ok(sheets)
}

/// 先以嚴格 UTF-8 解碼（可帶 BOM）；不是合法 UTF-8 就用系統 ANSI 字碼頁——
/// 繁中 Windows 上 Excel 匯出的 CSV 常是沒有 BOM 的 Big5。
pub fn decode_text(bytes: &[u8], fallback: &'static Encoding) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => fallback.decode(bytes).0.into_owned(),
    }
}

/// 系統 ANSI 字碼頁對應的編碼（非 Windows 或不認得時用 windows-1252）。
pub fn system_ansi_encoding() -> &'static Encoding {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetACP() -> u32;
        }
        let codepage = unsafe { GetACP() };
        let label = match codepage {
            950 => "big5",
            936 => "gbk",
            932 => "shift_jis",
            949 => "euc-kr",
            874 => "windows-874",
            1250..=1258 => return Encoding::for_label(format!("windows-{codepage}").as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252),
            _ => "windows-1252",
        };
        Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
    }
    #[cfg(not(windows))]
    {
        encoding_rs::WINDOWS_1252
    }
}

fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => format_number(*f),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(d) if d.time() == chrono::NaiveTime::MIN => d.format("%Y-%m-%d").to_string(),
            Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
            None => format_number(dt.as_f64()),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// 最多 10 位小數、去掉多餘的 0（255.5、1600，而不是 1600.0000000001）。
fn format_number(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(format_number(1600.0), "1600");
        assert_eq!(format_number(255.5), "255.5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
    }

    #[test]
    fn big5_fallback_and_utf8_bom() {
        let (big5, _, _) = encoding_rs::BIG5.encode("名稱,數值\n甲,1\n");
        assert_eq!(decode_text(&big5, encoding_rs::BIG5), "名稱,數值\n甲,1\n");
        assert_eq!(decode_text("\u{FEFF}a,b".as_bytes(), encoding_rs::BIG5), "a,b");
    }
}
