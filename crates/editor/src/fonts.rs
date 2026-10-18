//! 系統已安裝的字型家族（字型對話框的清單）。

use std::collections::BTreeSet;

use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};

unsafe extern "system" fn collect(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _kind: u32, lparam: LPARAM) -> i32 {
    unsafe {
        let set = &mut *(lparam.0 as *mut BTreeSet<String>);
        let face = &(*lf).lfFaceName;
        let len = face.iter().position(|&c| c == 0).unwrap_or(face.len());
        let name = String::from_utf16_lossy(&face[..len]);
        // '@' 開頭的是直排字型
        if !name.is_empty() && !name.starts_with('@') {
            set.insert(name);
        }
    }
    1
}

pub fn installed_families() -> Vec<String> {
    let mut set = BTreeSet::new();
    unsafe {
        let hdc = GetDC(None);
        let lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        EnumFontFamiliesExW(hdc, &lf, Some(collect), LPARAM(&mut set as *mut _ as isize), 0);
        ReleaseDC(None, hdc);
    }
    set.into_iter().collect()
}
