//! 使用者設定，存在 %APPDATA%\MarkForge\editor.json（C# 版沒有保存設定，每次啟動都回到預設）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub dark: bool,
    pub editor_font: String,
    pub editor_font_size: f32,
    pub preview_font: String,
    pub preview_font_size: f32,
    /// 預覽閱讀寬度（px），0 = 不限制
    pub preview_width: u32,
    pub show_editor: bool,
    pub show_preview: bool,
    /// 編輯區佔的寬度比例
    pub split: f32,
    pub window_width: u32,
    pub window_height: u32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            editor_font: "Consolas".into(),
            editor_font_size: 15.0,
            preview_font: markforge_core::template::DEFAULT_FONT_FAMILY.into(),
            preview_font_size: markforge_core::template::DEFAULT_FONT_SIZE,
            preview_width: markforge_core::template::DEFAULT_WIDTH_PX,
            show_editor: true,
            show_preview: true,
            split: 0.5,
            window_width: 1280,
            window_height: 820,
            maximized: false,
        }
    }
}

pub const WIDTH_PRESETS: &[(u32, &str)] =
    &[(860, "860px（預設）"), (1000, "1000px"), (1200, "1200px"), (1400, "1400px"), (0, "不限制寬度")];

fn settings_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("MarkForge").join("editor.json"))
}

impl Settings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
