//! 以真實樣本測試轉換（樣本由 tools/fixtures/make_fixtures.ps1 以 Word、LibreOffice、openpyxl 產生）。

use std::path::PathBuf;

use markforge_core::import::{clipboard, html_to_markdown, spreadsheet, table};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn convert(html: &str) -> String {
    html_to_markdown(html, &mut |_| None)
}

/// Word 剪貼簿的原始 CF_HTML：標頭位移不可信、樣式表藏在條件註解裡。
#[test]
fn word_clipboard_html() {
    let raw = std::fs::read(fixture("word_clipboard.html")).unwrap();
    let cf = clipboard::decode_cf_html(&raw);
    let html = clipboard::extract(&cf);
    assert!(clipboard::is_from_document_app(html));
    let md = convert(html);
    println!("{md}");

    let expected_lines = [
        "# 專案年度報告",          // Title 樣式（MsoTitle，字級在條件註解內的樣式表）
        "# 第一章 概述",           // 標題 1
        "# 手動放大的標題文字",     // 22pt ÷ 12pt ≥ 1.75
        "## 第二節 細項",          // 標題 2
        "#### 稍微放大的小標題",    // 14pt ÷ 12pt ≥ 1.12，粗體符號移除
        "- 第一個項目",
        "- 第二個項目",
        "1. 步驟一",               // 與項目符號清單是不同的 Word 清單，重新編號
        "2. 步驟二",
        "| 品項       | 數量 | 單價 |",
        "| 香蕉\\|芭樂 |  200 |    8 |",
        "結尾段落。",
    ];
    for line in expected_lines {
        assert!(md.lines().any(|l| l == line), "missing line {line:?} in:\n{md}");
    }
    // 粗體／斜體、跳脫、中文之間的換行不可變成空格
    assert!(md.contains("包含 **粗體文字** 與 *斜體文字*"), "{md}");
    assert!(md.contains("snake_case 與 2\\*3"), "{md}");
    assert!(md.contains("確認 Word 在輸出 HTML 時自動換行不會在中文字之間插入多餘的空白字元。"), "{md}");
}

#[test]
fn libreoffice_writer_html() {
    let html = std::fs::read_to_string(fixture("libreoffice_writer.html")).unwrap();
    let md = convert(&html);
    println!("{md}");
    let expected_lines = [
        "# 專案年度報告",          // Heading 1
        "包含 **粗體文字** 與 *斜體文字*。",
        "# 手動放大的標題文字",     // 22pt；內文沒有明確字級 → 以 12pt 計
        "## 第二節 細項",          // Heading 2
        "#### 稍微放大的小標題",    // 14pt
        "- 第一個項目",
        "- 第二個項目",
        "  - 巢狀子項目",
        "1. 步驟一",
        "2. 步驟二",
        "| 香蕉\\|芭樂 |  200 |",
        "結尾段落。",
    ];
    for line in expected_lines {
        assert!(md.lines().any(|l| l == line || l.ends_with(line)), "missing line {line:?} in:\n{md}");
    }
}

#[test]
fn xlsx_and_ods_workbooks() {
    for name in ["book.xlsx", "book.ods"] {
        let sheets = spreadsheet::read(&fixture(name)).unwrap();
        let names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["銷售", "庫存", "空白"], "{name}");

        let sales = table::build(&sheets[0].rows).unwrap();
        assert_eq!(
            sales,
            "| 日期       | 品項           | 數量 |  金額 |\n\
             | ---------- | -------------- | ---: | ----: |\n\
             | 2026-09-01 | 蘋果           |   10 | 255.5 |\n\
             | 2026-09-02 | 香蕉\\|芭樂     |  200 |  1600 |\n\
             |            | 多行<br>儲存格 |    3 |       |\n",
            "{name}"
        );
        assert!(table::build(&sheets[2].rows).is_none(), "{name}: empty sheet");
    }
}

#[test]
fn big5_csv() {
    let bytes = std::fs::read(fixture("data_big5.csv")).unwrap();
    let text = spreadsheet::decode_text(&bytes, encoding_rs::BIG5);
    let rows = table::parse_delimited(&text, table::guess_delimiter(&text));
    assert_eq!(
        table::build(&rows).unwrap(),
        "| 名稱 | 數值 | 備註     |\n| ---- | ---: | -------- |\n| 甲   |    1 | 含,逗號  |\n| 乙   |    2 | 含\"引號\" |\n"
    );
}
