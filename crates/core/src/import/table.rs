//! 格狀資料（CSV／TSV 文字、剪貼簿、試算表）→ GFM 表格。
//!
//! 第一列當表頭（GFM 沒有無表頭的表格）；欄位依顯示寬度補齊（中文算兩格），讓編輯區的
//! 原始碼也整齊；整欄皆為數字時靠右對齊。

use unicode_width::UnicodeWidthStr;

/// 建立表格；去除空白列／欄後沒有內容時回傳 `None`。
pub fn build<S: AsRef<str>>(raw_rows: &[Vec<S>]) -> Option<String> {
    let rows = normalize(raw_rows);
    if rows.is_empty() {
        return None;
    }
    let columns = rows[0].len();
    let cells: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| escape_cell(c)).collect()).collect();

    let right_align: Vec<bool> = (0..columns)
        .map(|c| {
            let body: Vec<&String> = cells.iter().skip(1).map(|r| &r[c]).filter(|v| !v.is_empty()).collect();
            !body.is_empty() && body.iter().all(|v| is_numeric(v))
        })
        .collect();

    let mut widths = vec![3usize; columns];
    for row in &cells {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(display_width(cell));
        }
    }

    let mut out = String::new();
    append_row(&mut out, &cells[0], &widths, &right_align);
    out.push('|');
    for c in 0..columns {
        out.push(' ');
        if right_align[c] {
            out.push_str(&"-".repeat(widths[c] - 1));
            out.push(':');
        } else {
            out.push_str(&"-".repeat(widths[c]));
        }
        out.push_str(" |");
    }
    out.push('\n');
    for row in cells.iter().skip(1) {
        append_row(&mut out, row, &widths, &right_align);
    }
    Some(out)
}

/// 解析分隔文字（RFC 4180 引號規則：`"a ""b"" c"`、引號內可換行）。
/// Excel 放到剪貼簿的就是這種格式，分隔符號為 Tab。
pub fn parse_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut field_started = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        if ch == '"' && !field_started {
            in_quotes = true;
            field_started = true;
        } else if ch == delimiter {
            row.push(std::mem::take(&mut field));
            field_started = false;
        } else if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            field_started = false;
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(ch);
            field_started = true;
        }
    }
    if field_started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// 依第一行 `,`、Tab、`;` 出現次數猜 CSV 的分隔符號。
pub fn guess_delimiter(text: &str) -> char {
    let first = text.lines().next().unwrap_or("");
    [',', '\t', ';']
        .into_iter()
        .max_by_key(|d| first.chars().filter(|c| c == d).count())
        .unwrap_or(',')
}

/// 純文字看起來像複製來的格狀資料：至少兩行、每個非空行的 Tab 數相同且 > 0。
/// 行首不可是 Tab（避免把 Tab 縮排的程式碼當成表格），也不含引號。
pub fn looks_like_tsv(text: &str) -> bool {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.trim_end_matches('\n').split('\n').collect();
    if lines.len() < 2 {
        return false;
    }
    let mut tabs: Option<usize> = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with('\t') || line.contains('"') {
            return false;
        }
        let count = line.matches('\t').count();
        if count == 0 {
            return false;
        }
        match tabs {
            None => tabs = Some(count),
            Some(t) if t != count => return false,
            _ => {}
        }
    }
    tabs.is_some()
}

/// 等寬字型下的顯示寬度：東亞寬字元與全形字算兩格。
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn normalize<S: AsRef<str>>(raw_rows: &[Vec<S>]) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = raw_rows
        .iter()
        .map(|r| r.iter().map(|c| c.as_ref().trim().to_string()).collect())
        .collect();
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    while rows.first().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.remove(0);
    }
    if rows.is_empty() {
        return rows;
    }
    let mut columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    while columns > 0 && rows.iter().all(|r| r.len() < columns || r[columns - 1].is_empty()) {
        columns -= 1;
    }
    if columns == 0 {
        return Vec::new();
    }
    for row in &mut rows {
        row.resize(columns, String::new());
    }
    rows
}

fn escape_cell(value: &str) -> String {
    value.replace("\r\n", "\n").replace('|', "\\|").replace('\n', "<br>")
}

fn is_numeric(value: &str) -> bool {
    let v: String = value.chars().filter(|c| !matches!(c, ',' | '%' | '$')).collect();
    let v = v.trim();
    !v.is_empty() && v.parse::<f64>().is_ok_and(f64::is_finite)
}

fn append_row(out: &mut String, row: &[String], widths: &[usize], right_align: &[bool]) {
    out.push('|');
    for (c, cell) in row.iter().enumerate() {
        let pad = " ".repeat(widths[c] - display_width(cell));
        out.push(' ');
        if right_align[c] {
            out.push_str(&pad);
            out.push_str(cell);
        } else {
            out.push_str(cell);
            out.push_str(&pad);
        }
        out.push_str(" |");
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_clipboard_tsv_with_quoted_multiline_cell() {
        let text = "品項\t數量\t備註\r\n蘋果\t10\t\"第一行\n第二行\"\r\n香蕉|芭樂\t200\t\r\n";
        let table = build(&parse_delimited(text.trim_end_matches(['\r', '\n']), '\t')).unwrap();
        assert_eq!(
            table,
            "| 品項       | 數量 | 備註             |\n\
             | ---------- | ---: | ---------------- |\n\
             | 蘋果       |   10 | 第一行<br>第二行 |\n\
             | 香蕉\\|芭樂 |  200 |                  |\n"
        );
    }

    #[test]
    fn trims_empty_rows_and_columns() {
        let rows = vec![vec!["", "", ""], vec!["a", "b", ""], vec!["1", "2", ""], vec!["", "", ""]];
        // 數字欄靠右對齊，表頭也跟著靠右（與 C# 版相同）
        assert_eq!(build(&rows).unwrap(), "|   a |   b |\n| --: | --: |\n|   1 |   2 |\n");
        assert!(build(&[vec!["", ""]]).is_none());
    }

    #[test]
    fn tsv_detection() {
        assert!(looks_like_tsv("a\tb\n1\t2\n"));
        assert!(!looks_like_tsv("int a;\n\tb();\n"));
        assert!(!looks_like_tsv("only one line\twith tab"));
        assert!(!looks_like_tsv("a\tb\n1\t2\t3\n"));
    }

    #[test]
    fn delimiter_guess_and_csv_quotes() {
        assert_eq!(guess_delimiter("a;b;c\n1;2;3"), ';');
        let rows = parse_delimited("名稱,備註\n甲,\"含,逗號\"\n乙,\"含\"\"引號\"\"\"\n", ',');
        assert_eq!(rows[1], vec!["甲", "含,逗號"]);
        assert_eq!(rows[2], vec!["乙", "含\"引號\""]);
    }

    #[test]
    fn cjk_counts_double_width() {
        assert_eq!(display_width("中文ab"), 6);
    }
}
