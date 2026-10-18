# MarkForge（Rust 版）

Windows 桌面 Markdown 編輯器／檢視器／檔案總管預覽窗格。v2.0.0 以 Rust 全面改寫自 C#／.NET 8 版 MarkForge v1.7.0（<https://github.com/zon1002/MarkForge>），功能對等，不再需要 .NET——只需要 Microsoft Edge WebView2 Runtime（Windows 11 已內建）。

版本歷程見 [versions.md](versions.md)；第三方元件授權見 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 功能

- **MarkForge Editor**：CodeMirror 6 編輯區＋即時預覽，雙向同步捲動；格式工具列與快捷鍵；深色（預設）／淺色主題；預覽寬度、字型、字級可調；匯出 HTML（可獨立開啟）與 PDF。
- **貼上轉換**：Word／LibreOffice／Google 文件的內容依字級與標題樣式轉成標準 Markdown 標題、粗斜體、清單、表格；Excel／Calc 儲存格轉 Markdown 表格；截圖存成圖片附件。
- **拖入**：圖片、CSV／TSV／XLSX／XLS／ODS（轉表格，自動判斷 Big5／UTF-8 編碼）、Mermaid（`.mmd`）、Markdown 檔（開啟），插入在放開滑鼠的位置。
- **附件**：未存檔前圖片先暫存，存檔時複製到 `.md` 所在資料夾並改寫為相對路徑。
- **MarkForge Viewer**：唯讀檢視，檔案被修改時自動重新載入並保留捲動位置。
- **檔案總管預覽窗格**：選取 `.md`／`.markdown` 時直接在預覽窗格（`Alt+P`）顯示渲染結果，包含表格、程式碼上色、Mermaid 與相對路徑圖片；不連網、不執行文件內的腳本。

## 安裝

執行 `installer_output\MarkForge_Setup_v2.0.0.exe`（需要系統管理員權限：預覽處理常式必須註冊在 HKLM）。會直接升級已安裝的 v1.x。

## 架構

| crate | 說明 |
|-------|------|
| `crates/editor`（MarkForgeEditor.exe） | 編輯器：tao 視窗 + muda 原生選單 + wry（WebView2）；前端 CodeMirror 6（`crates/editor/web`） |
| `crates/viewer`（MarkForgeViewer.exe） | 檢視器：唯讀顯示，檔案被修改時自動重新載入（notify） |
| `crates/preview-handler`（MarkForgePreview.dll） | 檔案總管預覽處理常式（IPreviewHandler COM，純 Rust）；`preview_test` 不經登錄依檔案總管的呼叫順序測試 |
| `crates/preview-host`（MarkForgePreviewHost.exe） | 預覽窗格內的渲染程式：Win32 子視窗 + WebView2，嚴格 CSP |
| `crates/shell`（markforge-shell） | Editor／Viewer 共用的 Windows 整合：WebView2 擴充（拖入檔案路徑、另存 PDF）、字型清單、外部連結、診斷記錄 |
| `crates/core`（markforge-core） | Markdown 渲染（comrak）、HTML 頁面範本、離線資源（highlight.js、mermaid.js、CSS、預覽腳本）、貼上／拖入轉換（Word／LibreOffice／Google 文件 HTML、表格、試算表）、附件管理 |

所有命令都先經過前端（取得目前文字）再以 IPC 交給 Rust；網頁資源以 `include_bytes!` 嵌入，透過 `http://markforge.localhost/` 自訂協定提供。

## 建置

需求：Rust stable（`x86_64-pc-windows-msvc`）、Inno Setup 6（打包安裝程式時）。

```bash
cargo test --workspace
cargo run -p markforge-editor -- 文件.md
cargo run -p markforge-viewer -- 文件.md
```

發行版與安裝程式：

```bash
powershell -ExecutionPolicy Bypass -File publish.ps1
```

產生 `publish\`（四個發行檔）與 `installer_output\MarkForge_Setup_v<版本>.exe`；加上 `-SkipInstaller` 只建置不打包。

前端的 CodeMirror 6 已預先打包在 `crates/editor/web/vendor/codemirror.js`；需要更新時在 `tools/frontend` 執行 `npm install && npm run build`。

測試樣本（`crates/core/tests/fixtures`）由 `tools/fixtures/make_fixtures.ps1` 以實際的 Word、LibreOffice、openpyxl 產生。

## 疑難排解

- 診斷記錄：設定環境變數 `MARKFORGE_DEBUG_LOG=記錄檔路徑` 後，Editor 會把 IPC 訊息、貼上判斷與前端錯誤寫入該檔。
- 預覽窗格顯示「沒有預覽可用」：確認安裝時勾選了「在檔案總管預覽窗格顯示 Markdown」，並重新啟動檔案總管；錯誤記錄於 `%LOCALAPPDATA%\MarkForge\PreviewHost\error.log`。
- 開發時手動註冊預覽處理常式（系統管理員）：`regsvr32 target\release\MarkForgePreview.dll`，移除用 `regsvr32 /u`。
