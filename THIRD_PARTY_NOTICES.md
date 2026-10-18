# 第三方元件授權聲明

MarkForge 隨附或編譯進以下第三方元件，各元件的著作權屬於其原作者，依各自授權條款使用。

## 內嵌的前端資源

| 元件 | 版本 | 授權 | 來源 |
|------|------|------|------|
| CodeMirror 6（@codemirror/*、@lezer/*） | 6.x | MIT | <https://codemirror.net/> |
| Mermaid | 11.17 | MIT | <https://mermaid.js.org/> |
| highlight.js | 11.9 | BSD-3-Clause | <https://highlightjs.org/> |
| Bootstrap Icons（工具列圖示） | 1.x | MIT | <https://icons.getbootstrap.com/> |

## Rust crate（編譯進執行檔）

| crate | 授權 |
|-------|------|
| comrak | BSD-2-Clause |
| scraper、ego-tree | ISC |
| calamine | MIT |
| encoding_rs | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| tao | Apache-2.0 |
| wry | Apache-2.0 OR MIT |
| muda | Apache-2.0 OR MIT |
| rfd | MIT |
| webview2-com | MIT |
| windows、windows-core | MIT OR Apache-2.0 |
| notify | CC0-1.0 |
| image、png | MIT OR Apache-2.0 |
| serde、serde_json、regex、base64、chrono、unicode-width | MIT OR Apache-2.0 |
| raw-window-handle | MIT OR Apache-2.0 OR Zlib |

完整的相依清單（含間接相依）可用 `cargo tree` 檢視；各 crate 的授權全文隨其原始碼發布於 <https://crates.io/>。

## 執行階段相依

- Microsoft Edge WebView2 Runtime：由 Microsoft 依其授權條款散布，不隨本安裝程式附帶。
