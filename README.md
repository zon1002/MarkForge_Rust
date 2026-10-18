# MarkForge（Rust 版）

Windows 桌面 Markdown 編輯器／檢視器，以 Rust 改寫自 C#／.NET 8 版 MarkForge v1.7.0（<https://github.com/zon1002/MarkForge>）。

> 開發中。功能、建置方式與版本歷程會在各階段完成後補齊。

## 架構

| crate | 說明 |
|-------|------|
| `crates/editor`（MarkForgeEditor.exe） | 編輯器：tao 視窗 + muda 原生選單 + wry（WebView2）；前端 CodeMirror 6（`crates/editor/web`） |
| `crates/core`（markforge-core） | Markdown 渲染（comrak）、HTML 頁面範本、離線資源（highlight.js、mermaid.js、CSS、預覽腳本）、貼上／拖入轉換（Word／LibreOffice／Google 文件 HTML、表格、試算表） |

## 建置

```bash
cargo test -p markforge-core
cargo run -p markforge-editor -- 文件.md
```

前端的 CodeMirror 6 已預先打包在 `crates/editor/web/vendor/codemirror.js`；需要更新時在 `tools/frontend` 執行 `npm install && npm run build`。

診斷：設定環境變數 `MARKFORGE_DEBUG_LOG=記錄檔路徑` 後，Editor 會把 IPC 訊息、貼上判斷與前端錯誤寫入該檔。

測試樣本（`crates/core/tests/fixtures`）由 `tools/fixtures/make_fixtures.ps1` 以實際的 Word、LibreOffice、openpyxl 產生。
