# 版本說明 (Versions)

MarkForge Markdown 工具組的版本歷程，最新版本在最前。格式參考 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/)，
版本依 [語義化版本](https://semver.org/lang/zh-TW/) 命名：`主版號.次版號.修訂號`。

- v2 起為 Rust 版（本資料夾）；v1.x 為 C#／.NET 8 版，完整歷程見 <https://github.com/zon1002/MarkForge> 的 `versions.md`
- 以 Git 管理，每個版本都有對應的標籤（`git tag`）；開發過程依階段提交（`Phase 1`～`Phase 8`）

| 版本 | 日期 | 重點 |
|------|------|------|
| [v2.0.0](#v200--2026-10-19) | 2026-10-19 | 以 Rust 全面改寫，功能對等 v1.7.0，不再需要 .NET |
| v1.7.0 | 2026-09-24 | 檔案總管預覽窗格直接顯示 .md（C# 版） |
| v1.6.0 | 2026-09-24 | 預設深色、圖片／表格／Mermaid 拖入、Word／ODF 貼上轉 Markdown（C# 版） |
| v1.0.0～v1.5.1 | 2026-08 | 首次發布、同步捲動、預覽寬度等（C# 版） |

---

## [v2.0.0] — 2026-10-19

以 Rust 重寫整個工具組，功能與 v1.7.0 對等。安裝程式沿用相同的 AppId，直接覆蓋升級 v1.x。

### 重大變更

- **不再需要 .NET 8 Desktop Runtime**：只需要 Microsoft Edge WebView2 Runtime（Windows 11 已內建；安裝程式偵測不到時可自動下載）。
- **安裝目錄改為單一資料夾**：`MarkForgeEditor.exe`、`MarkForgeViewer.exe`、`MarkForgePreviewHost.exe`、`MarkForgePreview.dll` 放在 `{app}` 下；升級時自動移除 v1.x 的 `Editor\`、`Viewer\`、`PreviewHandler\` 子資料夾。
- **預覽處理常式改為純 Rust COM**（新 CLSID `{57C7730F-A427-425D-92B1-CE2797C1C9CA}`）；升級時移除 v1.7.0 C++ 處理常式（`{05E77F8A-…}`）的登錄。
- **設定檔**：Editor 設定存於 `%APPDATA%\MarkForge\editor.json`、Viewer 存於 `viewer.json`；v1.x 的設定不會自動移轉。

### 架構

| 元件 | 技術 |
|------|------|
| `markforge-core` | comrak（GFM、表格、工作清單、註腳、提示區塊 `> [!NOTE]`、emoji 短碼、CJK 強調）、HTML 範本、離線資源（highlight.js、Mermaid）、貼上／拖入轉換、附件管理 |
| `markforge-shell` | Editor／Viewer 共用：WebView2 拖入檔案路徑、PrintToPdf、字型列舉、外部連結、診斷記錄 |
| Editor | tao 視窗 + muda 原生選單（深色）+ wry（WebView2）；前端 CodeMirror 6 離線打包 |
| Viewer | 同上，唯讀；notify 監看檔案、200ms 去抖動自動重新載入 |
| 預覽窗格 | `MarkForgePreview.dll`（windows-rs `#[implement]` 的 IPreviewHandler）+ `MarkForgePreviewHost.exe`（純 Win32 子視窗 + WebView2，嚴格 CSP） |

### 功能（對等 v1.7.0）

- **編輯器**：CodeMirror 6 語法上色、搜尋取代、格式工具列與快捷鍵（粗體、斜體、標題、清單、程式碼、連結、表格…），編輯／預覽雙向同步捲動，顯示／隱藏窗格，預覽寬度（含自訂），字型與字級，深色／淺色主題（預設深色）。
- **貼上轉換**：直接讀取 Win32 剪貼簿——Word／LibreOffice／Google 文件的 HTML 依字級與標題樣式轉成 `#`～`######`、粗斜體、清單、表格；Excel／LibreOffice Calc 儲存格轉 Markdown 表格（中英文混排對齊）；截圖（PNG／DIB）存成圖片附件；複製的檔案依類型插入。
- **拖入**：圖片 → `![]()`；`.csv`／`.tsv`／`.xlsx`／`.xls`／`.ods` → 表格（CSV 自動判斷 UTF-8／Big5 等編碼與分隔符號）；`.mmd`／`.mermaid` → Mermaid 程式碼區塊；`.md` → 開啟。插入位置為放開滑鼠的位置。
- **附件**：未存檔前圖片暫存於 `%LOCALAPPDATA%\MarkForge`，存檔時複製到 `.md` 所在資料夾並改寫為相對路徑。
- **匯出**：HTML（資源內嵌，可獨立開啟）、PDF（以淺色版面列印，避免深色背景遺失後文字看不見）。
- **Viewer**：唯讀顯示、自動重新載入且保留捲動位置、匯出 PDF、深淺色切換。
- **檔案總管預覽窗格**：`.md`／`.markdown` 直接顯示渲染結果（表格、程式碼上色、Mermaid、相對路徑圖片），配色跟隨 Windows 應用程式模式；不連網、腳本與事件屬性被 CSP 阻擋，連結交給預設瀏覽器開啟；超過 8 MB 只顯示提示。

### 改善

- **啟動與體積**：單一原生執行檔，無 .NET 啟動成本；所有網頁資源以 `include_bytes!` 嵌入，透過 `http://markforge.localhost/` 自訂協定提供，不再解壓縮到磁碟。
- **預覽處理常式**：不再需要 C++ 墊片＋.NET 主程式兩段式架構；切換檔案時預覽程式收到 `WM_CLOSE` 即結束，另以計時器偵測父視窗消失，避免殘留程序。
- **原生選單可用鍵盤操作**：WebView2 取得焦點時，`Alt+F/E/I/V/H` 與 `F10` 仍可開啟選單。
- **測試**：`markforge-core` 31 個單元測試＋4 個以真實 Word／LibreOffice／Excel 剪貼簿內容產生的整合測試樣本（`tools/fixtures/make_fixtures.ps1`）。
- **診斷**：設定 `MARKFORGE_DEBUG_LOG=檔案路徑` 可記錄 IPC、貼上判斷與前端錯誤。

### 開發階段

| 階段 | 內容 |
|------|------|
| Phase 1 | 工作區骨架、comrak 渲染、HTML 範本與離線資源 |
| Phase 2 | 貼上／拖入轉換器（HTML→Markdown、表格、試算表）與真實樣本 |
| Phase 3 | 附件管理（DocumentAssets） |
| Phase 4 | CodeMirror 6 離線打包 |
| Phase 5 | MarkForge Editor |
| Phase 6 | MarkForge Viewer 與共用的 markforge-shell |
| Phase 7 | 檔案總管預覽窗格（純 Rust 處理常式＋預覽程式） |
| Phase 8 | 發行腳本、安裝程式、文件 |
