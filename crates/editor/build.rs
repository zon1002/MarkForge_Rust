// exe 圖示與版本資訊（檔案總管「內容」頁顯示）；圖示 id 1 也供視窗標題列使用。
// 另把 ../core/icons/*.svg 依各種 DPI 的選單圖示尺寸轉成 PNG，供原生選單使用（menu_icons.rs）。
use std::fmt::Write as _;
use std::path::Path;

/// 選單圖示（SM_CXSMICON）在 100%～300% 縮放下的尺寸
const SIZES: &[u32] = &[16, 20, 24, 28, 32, 40, 48];

fn main() {
    println!("cargo:rerun-if-changed=../core/assets/editor.ico");
    println!("cargo:rerun-if-changed=../core/icons");
    render_menu_icons();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("../core/assets/editor.ico", "1");
        res.set("FileDescription", "MarkForge Editor");
        res.set("ProductName", "MarkForge Markdown 工具組");
        res.set("CompanyName", "Fu Project");
        res.set("OriginalFilename", "MarkForgeEditor.exe");
        res.set_language(0x0404);
        if let Err(e) = res.compile() {
            println!("cargo:warning=無法加入 exe 圖示：{e}");
        }
    }
}

fn render_menu_icons() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let png_dir = Path::new(&out_dir).join("menu-icons");
    std::fs::create_dir_all(&png_dir).unwrap();

    let mut svgs: Vec<_> = std::fs::read_dir("../core/icons")
        .expect("找不到 ../core/icons")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .collect();
    svgs.sort();

    let mut code = String::from(
        "/// 一個圖示的各種尺寸：(邊長, PNG)\npub type IconVariants = &'static [(u32, &'static [u8])];\n\
         /// (圖示名稱, 各尺寸)，由 build.rs 產生\npub static MENU_ICONS: &[(&str, IconVariants)] = &[\n",
    );
    for svg in &svgs {
        let name = svg.file_stem().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(svg).unwrap();
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
            .unwrap_or_else(|e| panic!("{}: {e}", svg.display()));
        write!(code, "    (\"{name}\", &[").unwrap();
        for &size in SIZES {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
            let scale = size as f32 / tree.size().width();
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
            let file = png_dir.join(format!("{name}-{size}.png"));
            pixmap.save_png(&file).unwrap();
            let file = file.to_string_lossy().replace('\\', "/");
            write!(code, "({size}, include_bytes!(\"{file}\")), ").unwrap();
        }
        code.push_str("]),\n");
    }
    code.push_str("];\n");
    std::fs::write(Path::new(&out_dir).join("menu_icons.rs"), code).unwrap();
}
