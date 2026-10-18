// exe 圖示與版本資訊（檔案總管「內容」頁顯示）；圖示 id 1 也供視窗標題列使用。
fn main() {
    println!("cargo:rerun-if-changed=../core/assets/viewer.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("../core/assets/viewer.ico", "1");
        res.set("FileDescription", "MarkForge 檔案總管預覽");
        res.set("ProductName", "MarkForge Markdown 工具組");
        res.set("CompanyName", "Fu Project");
        res.set("OriginalFilename", "MarkForgePreviewHost.exe");
        res.set_language(0x0404);
        if let Err(e) = res.compile() {
            println!("cargo:warning=無法加入 exe 圖示：{e}");
        }
    }
}
