//! 讀取 Windows 剪貼簿的原始格式。
//!
//! 貼上時不用瀏覽器的 clipboardData：它只給 HTML 片段（Word 的樣式表在片段之外，
//! 標題字級會遺失），也拿不到 Excel 的試算表格式與檔案總管複製的檔案路徑。

use std::path::PathBuf;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::w;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;

/// 貼上當下剪貼簿裡我們在意的內容。
#[derive(Default)]
pub struct Snapshot {
    pub files: Vec<PathBuf>,
    /// Excel／LibreOffice Calc 提供的試算表格式（XML Spreadsheet、Csv）
    pub spreadsheet: bool,
    /// CF_HTML 原始內容（含標頭）
    pub html: Option<String>,
    pub text: Option<String>,
    /// 「PNG」格式（保留透明）
    pub png: Option<Vec<u8>>,
    /// CF_DIB（截圖工具常只放這個）
    pub dib: Option<Vec<u8>>,
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn open() -> Option<Guard> {
    // 其他程式可能正短暫佔用剪貼簿
    for _ in 0..20 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Some(Guard);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

fn available(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

fn bytes(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let handle: HANDLE = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let size = GlobalSize(global);
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(data)
    }
}

fn unicode_text() -> Option<String> {
    let raw = bytes(CF_UNICODETEXT)?;
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

fn dropped_files() -> Vec<PathBuf> {
    unsafe {
        let Ok(handle) = GetClipboardData(CF_HDROP) else { return Vec::new() };
        let hdrop = HDROP(handle.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        (0..count)
            .filter_map(|i| {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                (n > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
            })
            .collect()
    }
}

/// 讀取剪貼簿。`plain_only` 時只取文字（「貼上為純文字」）。
pub fn read(plain_only: bool) -> Option<Snapshot> {
    let _guard = open()?;
    let mut snap = Snapshot { text: unicode_text(), ..Snapshot::default() };
    if plain_only {
        return Some(snap);
    }
    unsafe {
        let html_format = RegisterClipboardFormatW(w!("HTML Format"));
        let png_format = RegisterClipboardFormatW(w!("PNG"));
        let xml_spreadsheet = RegisterClipboardFormatW(w!("XML Spreadsheet"));
        let csv = RegisterClipboardFormatW(w!("Csv"));

        if available(CF_HDROP) {
            snap.files = dropped_files();
        }
        snap.spreadsheet = available(xml_spreadsheet) || available(csv);
        if available(html_format) {
            snap.html = bytes(html_format).map(|b| markforge_core::import::clipboard::decode_cf_html(&b));
        }
        if available(png_format) {
            snap.png = bytes(png_format);
        }
        if snap.png.is_none() && available(CF_DIB) {
            snap.dib = bytes(CF_DIB);
        }
    }
    Some(snap)
}

/// CF_DIB（BITMAPINFO + 像素）→ PNG。前面補上 BITMAPFILEHEADER 交給 image 解 BMP。
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([dib[o], dib[o + 1], dib[o + 2], dib[o + 3]]);
    let header_size = u32_at(0) as usize;
    let bit_count = u16::from_le_bytes([dib[14], dib[15]]) as u32;
    let compression = u32_at(16);
    let colors_used = u32_at(32) as usize;
    let palette = if colors_used > 0 { colors_used } else if bit_count <= 8 { 1usize << bit_count } else { 0 };
    // BI_BITFIELDS 搭配 40 位元組標頭時，色彩遮罩緊接在標頭後
    let masks = if compression == 3 && header_size == 40 { 12 } else { 0 };
    let pixel_offset = 14 + header_size + palette * 4 + masks;

    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);

    let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?;
    // 32 位元截圖的 alpha 常是全 0，轉成不透明的 RGB 才不會變成整張透明
    let img = image::DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
