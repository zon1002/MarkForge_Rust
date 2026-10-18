//! 外部內容 → Markdown：剪貼簿 HTML（Word／LibreOffice／Google 文件）、表格、試算表檔案。
//! 與作業系統無關的純轉換邏輯；讀取剪貼簿、檔案拖放在 Editor 端處理。

pub mod clipboard;
pub mod html;
pub mod spreadsheet;
pub mod table;

pub use html::html_to_markdown;
