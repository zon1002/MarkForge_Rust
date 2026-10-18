# MarkForge（Rust 版）

Windows 桌面 Markdown 編輯器／檢視器，以 Rust 改寫自 C#／.NET 8 版 MarkForge v1.7.0（<https://github.com/zon1002/MarkForge>）。

> 開發中。功能、建置方式與版本歷程會在各階段完成後補齊。

## 架構

| crate | 說明 |
|-------|------|
| `crates/core`（markforge-core） | Markdown 渲染（comrak）、HTML 頁面範本、離線資源（highlight.js、mermaid.js、CSS、預覽腳本） |

## 建置

```bash
cargo test -p markforge-core
```
