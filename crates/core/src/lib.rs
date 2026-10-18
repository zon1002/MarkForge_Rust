//! MarkForge 共用核心，Editor、Viewer 與檔案總管預覽程式都使用這一份：
//!   * [`render`]：Markdown → HTML（comrak，GFM + emoji + Mermaid 區塊 + 原始碼位置標記）
//!   * [`template`]：完整 HTML 頁面（WebView 內顯示，或內嵌所有資源的獨立匯出檔）
//!   * [`import`]：剪貼簿 HTML（Word／LibreOffice／Google 文件）、表格、試算表 → Markdown
//!   * [`attachments`]：拖入／貼上但尚未存檔的圖片與附件，存檔時複製到 .md 所在資料夾
//!   * [`assets`]：內嵌的離線資源（highlight.js、mermaid.js、CSS、頁面腳本），執行期不連外部 CDN

pub mod assets;
pub mod attachments;
pub mod import;
pub mod render;
pub mod template;

pub use render::render_body;
pub use template::{AssetMode, PageOptions, page_html};
