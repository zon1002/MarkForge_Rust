//! 原生彈出選單（選單列下拉、右鍵選單）的深色／淺色。
//!
//! muda 只自行繪製選單列本身；下拉的彈出選單由系統繪製，顏色取決於整個程序的
//! 「偏好應用程式模式」。tao 啟動時設成 AllowDark（跟著 Windows 設定），所以 Windows
//! 是深色時，程式切到淺色主題後彈出選單仍是深色。這裡依程式自己的主題強制設定
//! ForceDark／ForceLight，再清除選單的佈景主題快取讓新設定生效。
//!
//! 使用 uxtheme.dll 未公開的序號匯出 135（SetPreferredAppMode）與 136（FlushMenuThemes），
//! Windows 10 1903（組建 18362）起提供；找不到時什麼都不做（維持系統預設）。

use std::sync::LazyLock;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

#[repr(C)]
#[allow(dead_code)]
enum PreferredAppMode {
    Default,
    AllowDark,
    ForceDark,
    ForceLight,
}

type SetPreferredAppMode = unsafe extern "system" fn(PreferredAppMode) -> PreferredAppMode;
type FlushMenuThemes = unsafe extern "system" fn();

struct UxTheme {
    set_preferred_app_mode: Option<SetPreferredAppMode>,
    flush_menu_themes: Option<FlushMenuThemes>,
}

static UXTHEME: LazyLock<UxTheme> = LazyLock::new(|| unsafe {
    let module = LoadLibraryW(w!("uxtheme.dll")).unwrap_or(HMODULE::default());
    let ordinal = |n: usize| {
        if module.is_invalid() { None } else { GetProcAddress(module, PCSTR(n as *const u8)) }
    };
    // 舊版 Windows 的序號 135 是 AllowDarkModeForApp(bool)，參數意義不同，不呼叫
    let supported = windows_build() >= 18362;
    UxTheme {
        set_preferred_app_mode: supported.then(|| ordinal(135)).flatten().map(|f| std::mem::transmute(f)),
        flush_menu_themes: supported.then(|| ordinal(136)).flatten().map(|f| std::mem::transmute(f)),
    }
});

/// Windows 組建編號（RtlGetVersion 不受相容性設定影響）。
fn windows_build() -> u32 {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    if unsafe { RtlGetVersion(&mut info) }.is_ok() { info.dwBuildNumber } else { 0 }
}

/// 讓之後開啟的原生彈出選單使用深色或淺色。
pub fn set_popup_menu_theme(dark: bool) {
    let ux = &*UXTHEME;
    if let (Some(set_mode), Some(flush)) = (ux.set_preferred_app_mode, ux.flush_menu_themes) {
        unsafe {
            set_mode(if dark { PreferredAppMode::ForceDark } else { PreferredAppMode::ForceLight });
            flush();
        }
    }
}
