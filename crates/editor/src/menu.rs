//! 原生選單列（muda）。選單項目的 id 就是前端的指令名稱：點選後交給前端 `app.command(id)`，
//! 由前端帶著目前的編輯內容再呼叫 Rust——跟快捷鍵、工具列走同一條路。
//!
//! 快捷鍵只「顯示」在選單上，實際由前端處理：WebView2 取得焦點後，按鍵不會經過本程式的
//! 訊息迴圈，選單的加速鍵表永遠收不到。

use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::settings::{Settings, WIDTH_PRESETS};

pub struct AppMenu {
    pub menu: Menu,
    pub show_editor: CheckMenuItem,
    pub show_preview: CheckMenuItem,
    pub dark: CheckMenuItem,
    pub widths: Vec<(u32, CheckMenuItem)>,
    pub width_custom: CheckMenuItem,
}

fn item(id: &str, text: &str, shortcut: Option<&str>) -> MenuItem {
    // 只用來顯示：解析失敗就不顯示快捷鍵
    let accel = shortcut.and_then(|s| s.parse::<Accelerator>().ok());
    MenuItem::with_id(id, text, true, accel)
}

fn check(id: &str, text: &str, checked: bool) -> CheckMenuItem {
    CheckMenuItem::with_id(id, text, true, checked, None)
}

impl AppMenu {
    pub fn build(settings: &Settings) -> muda::Result<Self> {
        let sep = PredefinedMenuItem::separator;

        let file = Submenu::with_items(
            "檔案(&F)",
            true,
            &[
                &item("file.new", "新增(&N)", Some("Ctrl+N")),
                &item("file.open", "開啟(&O)...", Some("Ctrl+O")),
                &item("file.save", "儲存(&S)", Some("Ctrl+S")),
                &item("file.saveAs", "另存新檔(&A)...", Some("Ctrl+Shift+S")),
                &sep(),
                &item("file.exportHtml", "匯出為 HTML(&E)...", None),
                &item("file.exportPdf", "另存為 PDF(&P)...", None),
                &sep(),
                &item("app.exit", "結束(&X)", None),
            ],
        )?;

        let edit = Submenu::with_items(
            "編輯(&E)",
            true,
            &[
                &item("edit.undo", "復原(&U)", Some("Ctrl+Z")),
                &item("edit.redo", "重做(&R)", Some("Ctrl+Y")),
                &sep(),
                &item("edit.cut", "剪下(&T)", Some("Ctrl+X")),
                &item("edit.copy", "複製(&C)", Some("Ctrl+C")),
                &item("edit.paste", "貼上(&P)", Some("Ctrl+V")),
                &item("edit.pastePlain", "貼上為純文字(&L)", Some("Ctrl+Shift+V")),
                &item("edit.selectAll", "全選(&A)", Some("Ctrl+A")),
                &sep(),
                &item("edit.find", "尋找／取代(&F)...", Some("Ctrl+F")),
                &sep(),
                &item("fmt.bold", "粗體(&B)", Some("Ctrl+B")),
                &item("fmt.italic", "斜體(&I)", Some("Ctrl+I")),
                &item("fmt.link", "插入連結(&K)", Some("Ctrl+K")),
            ],
        )?;

        let insert = Submenu::with_items(
            "插入(&I)",
            true,
            &[
                &item("insert.image", "圖片檔案(&I)...", None),
                &item("insert.table", "表格檔案 (Excel／CSV／ODS)(&T)...", None),
                &item("insert.mermaid", "Mermaid 圖表檔案(&M)...", None),
                &item("insert.attachment", "附件檔案(&A)...", None),
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
                &item("view.editorFont", "編輯器字型(&F)...", None),
                &item("view.previewFont", "預覽字型(&G)...", None),
                &width_menu,
            ],
        )?;

        let help = Submenu::with_items("說明(&H)", true, &[&item("help.about", "關於 MarkForge(&A)", None)])?;

        let menu = Menu::with_items(&[&file, &edit, &insert, &view, &help])?;
        let me = AppMenu { menu, show_editor, show_preview, dark, widths, width_custom };
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
