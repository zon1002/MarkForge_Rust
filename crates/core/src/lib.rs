//! MarkForge 共用核心，Editor、Viewer 與檔案總管預覽程式都使用這一份：
//!   * [`render`]：Markdown → HTML（comrak，GFM + emoji + Mermaid 區塊 + 原始碼位置標記）
//!   * [`template`]：完整 HTML 頁面（WebView 內顯示，或內嵌所有資源的獨立匯出檔）
//!   * [`assets`]：內嵌的離線資源（highlight.js、mermaid.js、CSS、頁面腳本），執行期不連外部 CDN

pub mod assets;
pub mod render;
pub mod template;

pub use render::render_body;
pub use template::{AssetMode, PageOptions, page_html};
