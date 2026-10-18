//! 原生選單列（muda）。選單項目的 id 就是前端的指令名稱：點選後交給前端 `app.command(id)`，
//! 由前端帶著目前的編輯內容再呼叫 Rust——跟快捷鍵、工具列走同一條路。
//!
//! 快捷鍵只「顯示」在選單上，實際由前端處理：WebView2 取得焦點後，按鍵不會經過本程式的
//! 訊息迴圈，選單的加速鍵表永遠收不到。

use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::menu_icons::MenuIcons;
use crate::settings::{Settings, WIDTH_PRESETS};

pub struct AppMenu {
    pub menu: Menu,
    /// 選單文字 → 彩色圖示（menu_icons.rs 依視窗 DPI 套用）
    pub icons: MenuIcons,
    pub show_editor: CheckMenuItem,
    pub show_preview: CheckMenuItem,
    pub dark: CheckMenuItem,
    pub widths: Vec<(u32, CheckMenuItem)>,
    pub width_custom: CheckMenuItem,
}

/// 建立選單項目並記下它的圖示（`icon` 是 crates/core/icons 下的檔名）。
fn item(icons: &mut MenuIcons, id: &str, text: &str, shortcut: Option<&str>, icon: &'static str) -> MenuItem {
    // 只用來顯示：解析失敗就不顯示快捷鍵
    let accel = shortcut.and_then(|s| s.parse::<Accelerator>().ok());
    icons.register(text, icon);
    MenuItem::with_id(id, text, true, accel)
}

fn check(id: &str, text: &str, checked: bool) -> CheckMenuItem {
    CheckMenuItem::with_id(id, text, true, checked, None)
}

impl AppMenu {
    pub fn build(settings: &Settings) -> muda::Result<Self> {
        let sep = PredefinedMenuItem::separator;
        let mut icons = MenuIcons::default();
        let ic = &mut icons;

        let file = Submenu::with_items(
            "檔案(&F)",
            true,
            &[
                &item(ic, "file.new", "新增(&N)", Some("Ctrl+N"), "file-new"),
                &item(ic, "file.open", "開啟(&O)...", Some("Ctrl+O"), "file-open"),
                &item(ic, "file.save", "儲存(&S)", Some("Ctrl+S"), "file-save"),
                &item(ic, "file.saveAs", "另存新檔(&A)...", Some("Ctrl+Shift+S"), "file-save-as"),
                &sep(),
                &item(ic, "file.exportHtml", "匯出為 HTML(&E)...", None, "export-html"),
                &item(ic, "file.exportPdf", "另存為 PDF(&P)...", None, "export-pdf"),
                &sep(),
                &item(ic, "app.exit", "結束(&X)", None, "app-exit"),
            ],
        )?;

        let edit = Submenu::with_items(
            "編輯(&E)",
            true,
            &[
                &item(ic, "edit.undo", "復原(&U)", Some("Ctrl+Z"), "edit-undo"),
                &item(ic, "edit.redo", "重做(&R)", Some("Ctrl+Y"), "edit-redo"),
                &sep(),
                &item(ic, "edit.cut", "剪下(&T)", Some("Ctrl+X"), "edit-cut"),
                &item(ic, "edit.copy", "複製(&C)", Some("Ctrl+C"), "edit-copy"),
                &item(ic, "edit.paste", "貼上(&P)", Some("Ctrl+V"), "edit-paste"),
                &item(ic, "edit.pastePlain", "貼上為純文字(&L)", Some("Ctrl+Shift+V"), "edit-paste-plain"),
                &item(ic, "edit.selectAll", "全選(&A)", Some("Ctrl+A"), "edit-select-all"),
                &sep(),
                &item(ic, "edit.find", "尋找／取代(&F)...", Some("Ctrl+F"), "edit-find"),
                &sep(),
                &item(ic, "fmt.bold", "粗體(&B)", Some("Ctrl+B"), "fmt-bold"),
                &item(ic, "fmt.italic", "斜體(&I)", Some("Ctrl+I"), "fmt-italic"),
                &item(ic, "fmt.link", "插入連結(&K)", Some("Ctrl+K"), "fmt-link"),
            ],
        )?;

        let insert = Submenu::with_items(
            "插入(&I)",
            true,
            &[
                &item(ic, "insert.image", "圖片檔案(&I)...", None, "insert-image"),
                &item(ic, "insert.table", "表格檔案 (Excel／CSV／ODS)(&T)...", None, "insert-table"),
                &item(ic, "insert.mermaid", "Mermaid 圖表檔案(&M)...", None, "insert-mermaid"),
                &item(ic, "insert.attachment", "附件檔案(&A)...", None, "insert-attachment"),
            ],
        )?;

        let dark = check("view.toggleTheme", "深色主題(&D)", settings.dark);
        let show_editor = check("view.toggleEditor", "顯示編輯器(&E)", settings.show_editor);
        let show_preview = check("view.togglePreview", "顯示預覽(&P)", settings.show_preview);

        let widths: Vec<(u32, CheckMenuItem)> = WIDTH_PRESETS
            .iter()
            .map(|(w, label)| (*w, check(&format!("view.width:{w}"), label, settings.preview_width == *w)))
            .collect();
        let width_custom = check("view.widthCustom", "自訂寬度(&C)...", false);
        let width_menu = Submenu::new("預覽寬度(&W)", true);
        ic.register("預覽寬度(&W)", "view-width");
        for (_, w) in &widths {
            width_menu.append(w)?;
        }
        width_menu.append(&sep())?;
        width_menu.append(&width_custom)?;

        let view = Submenu::with_items(
            "檢視(&V)",
            true,
            &[
                &dark,
                &sep(),
                &show_editor,
                &show_preview,
                &sep(),
                &item(ic, "view.editorFont", "編輯器字型(&F)...", None, "view-editor-font"),
                &item(ic, "view.previewFont", "預覽字型(&G)...", None, "view-preview-font"),
                &width_menu,
            ],
        )?;

        let help = Submenu::with_items("說明(&H)", true, &[&item(ic, "help.about", "關於 MarkForge(&A)", None, "help-about")])?;

        let menu = Menu::with_items(&[&file, &edit, &insert, &view, &help])?;
        let me = AppMenu { menu, icons, show_editor, show_preview, dark, widths, width_custom };
        me.sync(settings);
        Ok(me)
    }

    /// 勾選狀態跟著設定走（設定可能從前端或對話框改變）。
    pub fn sync(&self, settings: &Settings) {
        self.dark.set_checked(settings.dark);
        self.show_editor.set_checked(settings.show_editor);
        self.show_preview.set_checked(settings.show_preview);
        let mut matched = false;
        for (w, item) in &self.widths {
            let on = *w == settings.preview_width;
            item.set_checked(on);
            matched |= on;
        }
        self.width_custom.set_checked(!matched);
        self.width_custom.set_text(if matched {
            "自訂寬度(&C)...".to_string()
        } else {
            format!("自訂寬度(&C)...（目前 {}px）", settings.preview_width)
        });
    }
}
