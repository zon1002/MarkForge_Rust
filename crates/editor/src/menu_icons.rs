//! 原生選單的彩色圖示。
//!
//! 圖示來源是 crates/core/icons/*.svg（與工具列共用），build.rs 先轉成各種尺寸的 PNG。
//! 不用 muda 的 `IconMenuItem`：它固定把圖示畫成 16×16，高 DPI 下會模糊；這裡依視窗 DPI
//! 選擇對應尺寸，做成 32 位元預乘 alpha 的 DIB，直接設定到選單項目（`hbmpItem`）。
//! 項目以選單文字（Tab 前的部分）對應，視窗搬到不同 DPI 的螢幕時重新套用。

use std::ffi::c_void;

use image::imageops::FilterType;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawMenuBar, GetMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW, MIIM_BITMAP, MIIM_STRING,
    MIIM_SUBMENU, SM_CXSMICON, SetMenuItemInfoW,
};
use windows::core::PWSTR;

include!(concat!(env!("OUT_DIR"), "/menu_icons.rs"));

#[derive(Default)]
pub struct MenuIcons {
    /// (選單文字, 圖示名稱)
    labels: Vec<(String, &'static str)>,
    /// 目前設定在選單上的點陣圖；選單存在期間必須保留，換 DPI 時才釋放
    bitmaps: Vec<HBITMAP>,
}

impl MenuIcons {
    pub fn register(&mut self, text: &str, icon: &'static str) {
        self.labels.push((text.to_string(), icon));
    }

    /// 依視窗目前的 DPI 把圖示套用到視窗選單列的所有子選單。
    pub fn apply(&mut self, hwnd: HWND) {
        unsafe {
            let dpi = GetDpiForWindow(hwnd).max(96);
            let size = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16) as u32;
            let old = std::mem::take(&mut self.bitmaps);
            let bar = GetMenu(hwnd);
            if !bar.is_invalid() {
                self.walk(bar, size);
            }
            let _ = DrawMenuBar(hwnd);
            for b in old {
                let _ = DeleteObject(b.into());
            }
        }
    }

    unsafe fn walk(&mut self, menu: HMENU, size: u32) {
        let count = unsafe { GetMenuItemCount(Some(menu)) };
        for pos in 0..count.max(0) as u32 {
            let mut buf = [0u16; 256];
            let mut info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING | MIIM_SUBMENU,
                dwTypeData: PWSTR(buf.as_mut_ptr()),
                cch: buf.len() as u32 - 1,
                ..Default::default()
            };
            if unsafe { GetMenuItemInfoW(menu, pos, true, &mut info) }.is_err() {
                continue;
            }
            let text = String::from_utf16_lossy(&buf[..(info.cch as usize).min(buf.len())]);
            let label = text.split('\t').next().unwrap_or_default();
            let icon = self.labels.iter().find(|(l, _)| l == label).map(|(_, i)| *i);
            if let Some(bitmap) = icon.and_then(|i| create_bitmap(i, size)) {
                let set = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_BITMAP,
                    hbmpItem: bitmap,
                    ..Default::default()
                };
                let _ = unsafe { SetMenuItemInfoW(menu, pos, true, &set) };
                self.bitmaps.push(bitmap);
            }
            if !info.hSubMenu.is_invalid() {
                unsafe { self.walk(info.hSubMenu, size) };
            }
        }
    }
}

/// 取最接近的預先轉好的尺寸（必要時再縮放），做成由上而下、BGRA 預乘 alpha 的 32 位元 DIB。
fn create_bitmap(icon: &str, size: u32) -> Option<HBITMAP> {
    let (_, variants) = MENU_ICONS.iter().find(|(name, _)| *name == icon)?;
    let (_, png) = variants.iter().min_by_key(|(s, _)| s.abs_diff(size))?;
    let mut img = image::load_from_memory(png).ok()?.into_rgba8();
    if img.width() != size {
        img = image::imageops::resize(&img, size, size, FilterType::Lanczos3);
    }

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        return None;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize) };
    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = s.0;
        let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        d.copy_from_slice(&[pm(b), pm(g), pm(r), a]);
    }
    Some(bitmap)
}
