// 把 icons/*.svg（MarkForge 自行繪製的彩色圖示）合成 `color-icons.js`：
// `window.ICONS = { "名稱": "<svg…>" }`，Editor／Viewer 的工具列與彈出選單共用。
use std::fmt::Write as _;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");
    let mut entries: Vec<_> = std::fs::read_dir("icons")
        .expect("找不到 icons 資料夾")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .collect();
    entries.sort();

    let mut js = String::from("// 由 crates/core/build.rs 自 crates/core/icons/*.svg 產生（MarkForge 自行繪製的彩色圖示）。\nwindow.ICONS = {\n");
    for path in &entries {
        let name = path.file_stem().unwrap().to_string_lossy();
        let svg = std::fs::read_to_string(path).expect("無法讀取 SVG");
        let svg = svg.split_whitespace().collect::<Vec<_>>().join(" ");
        let escaped = svg.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(js, "  \"{name}\": \"{escaped}\",").unwrap();
    }
    js.push_str("};\n");

    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("color-icons.js");
    std::fs::write(out, js).expect("無法寫入 color-icons.js");
}
