//! MarkForge Editor：tao 視窗 + muda 原生選單 + wry（WebView2）。
//!
//! 介面（工具列、CodeMirror 編輯區、預覽、狀態列）都是 web/ 下的頁面，經自訂協定
//! `http://markforge.localhost/` 提供；Rust 負責 Markdown 渲染、檔案、對話框、剪貼簿、
//! 附件與匯出。所有指令（選單、快捷鍵、工具列）先到前端，前端帶著目前的編輯內容
//! 再以 IPC 呼叫 Rust，所以 Rust 永遠拿到最新的文字。

#![windows_subsystem = "windows"]

mod clipboard;
mod insert;
mod menu;
mod menu_icons;
mod settings;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use markforge_core::attachments::DocumentAssets;
use markforge_core::{AssetMode, PageOptions, assets, page_html, render_body};
use markforge_shell::webview2::{self, DROP_MESSAGE_PREFIX};
use markforge_shell::{debug_log, fonts, local_data_dir, open_external, percent_decode};
use muda::{MenuEvent, MenuTheme};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::Deserialize;
use serde_json::json;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::IconExtWindows;
use tao::window::{Icon, Theme, Window, WindowBuilder};
use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

use insert::{Paste, Snippet};
use menu::AppMenu;
use settings::Settings;

const APP_NAME: &str = "MarkForge";
const NEW_DOCUMENT: &str = "# 新文件\n\n開始輸入內容...\n";
const MARKDOWN_FILTER: (&str, &[&str]) = ("Markdown 檔案", &["md", "markdown"]);

pub enum UserEvent {
    Menu(String),
    Ipc(String),
    FilesDropped { paths: Vec<PathBuf>, x: f64, y: f64, target: String },
    PdfDone(Result<PathBuf, String>),
}

/// 前端 → Rust 的訊息。
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum Ipc {
    Ready,
    Render { id: u64, text: String },
    Dirty { dirty: bool },
    Command {
        name: String,
        #[serde(default)]
        arg: Option<String>,
        #[serde(default)]
        text: String,
    },
    Paste { plain: bool },
    OpenExternal { url: String },
    Settings { patch: serde_json::Value },
    /// 前端的錯誤訊息（寫入診斷記錄）
    Log { message: String },
    /// Alt+字母／F10：WebView2 有焦點時按鍵到不了原生選單，由前端轉來開啟選單
    MenuKey { key: String },
}

struct App {
    window: Window,
    webview: WebView,
    menu: AppMenu,
    proxy: EventLoopProxy<UserEvent>,
    settings: Settings,
    assets: Arc<Mutex<DocumentAssets>>,
    doc_path: Option<PathBuf>,
    dirty: bool,
    initial_file: Option<PathBuf>,
}

fn main() {
    let initial_file = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .map(|p| std::path::absolute(&p).unwrap_or(p));
    let settings = Settings::load();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(LogicalSize::new(settings.window_width, settings.window_height))
        .with_maximized(settings.maximized)
        .with_theme(Some(if settings.dark { Theme::Dark } else { Theme::Light }))
        .with_window_icon(Icon::from_resource(1, None).ok())
        .build(&event_loop)
        .expect("無法建立視窗");

    let mut menu = AppMenu::build(&settings).expect("無法建立選單");
    let hwnd = window_hwnd(&window);
    unsafe {
        let _ = menu.menu.init_for_hwnd(hwnd);
        let _ = menu.menu.set_theme_for_hwnd(hwnd, menu_theme(settings.dark));
    }
    menu.icons.apply(windows::Win32::Foundation::HWND(hwnd as _));
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e.id().0.clone()));
    }));

    let assets = Arc::new(Mutex::new(DocumentAssets::new(&std::env::temp_dir().join("MarkForge").join("Staging"))));
    let mut web_context = WebContext::new(local_data_dir().map(|d| d.join("Editor").join("WebView2")));
    let protocol_assets = assets.clone();
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_custom_protocol("markforge".into(), move |_id, request| serve(&request, &protocol_assets))
        .with_url("http://markforge.localhost/index.html")
        .with_ipc_handler(move |req: Request<String>| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(req.into_body()));
        })
        .with_navigation_handler(|url| url.starts_with("http://markforge.localhost/"))
        .with_new_window_req_handler(|url, _| {
            open_external(&url);
            wry::NewWindowResponse::Deny
        })
        .with_default_context_menus(false)
        .with_browser_accelerator_keys(false)
        .with_devtools(cfg!(debug_assertions))
        .with_background_color(if settings.dark { (13, 17, 23, 255) } else { (255, 255, 255, 255) })
        .build(&window)
        .expect("無法初始化 WebView2（請確認已安裝 Microsoft Edge WebView2 Runtime）");

    let drop_proxy = proxy.clone();
    match webview2::install_file_drop_hook(&webview, move |d| {
        let _ = drop_proxy.send_event(UserEvent::FilesDropped { paths: d.paths, x: d.x, y: d.y, target: d.target });
    }) {
        Ok(()) => debug_log("file drop hook installed"),
        Err(e) => debug_log(&format!("file drop hook failed: {e}")),
    }

    let mut app = App {
        window,
        webview,
        menu,
        proxy,
        settings,
        assets,
        doc_path: None,
        dirty: false,
        initial_file,
    };
    app.update_title();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => app.run_js("app.command('app.exit')"),
            // 搬到不同縮放比例的螢幕：選單圖示換成對應尺寸
            Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { .. }, .. } => {
                let hwnd = windows::Win32::Foundation::HWND(window_hwnd(&app.window) as _);
                app.menu.icons.apply(hwnd);
            }
            Event::UserEvent(UserEvent::Menu(id)) => app.run_js(&format!("app.command({})", js(&id))),
            Event::UserEvent(UserEvent::Ipc(body)) => {
                if app.handle_ipc(&body) {
                    app.remember_window();
                    app.settings.save();
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::FilesDropped { paths, x, y, target }) => app.files_dropped(paths, x, y, &target),
            Event::UserEvent(UserEvent::PdfDone(result)) => {
                app.run_js("app.pdfDone()");
                match result {
                    Ok(path) => app.info(&format!("已另存為 PDF：\n{}", path.display())),
                    Err(e) => app.warn("匯出失敗", &format!("無法匯出 PDF：\n{e}")),
                }
            }
            _ => {}
        }
    });
}

// ============================== 自訂協定 ==============================

macro_rules! web_file {
    ($name:literal) => {
        include_bytes!(concat!("../web/", $name)).as_slice()
    };
}

fn serve(request: &Request<Vec<u8>>, assets: &Arc<Mutex<DocumentAssets>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    let decoded = percent_decode(path);

    let found: Option<(Cow<'static, [u8]>, &str)> = match path {
        "/" | "/index.html" => Some((web_file!("index.html").into(), "text/html; charset=utf-8")),
        "/app.css" => Some((web_file!("app.css").into(), "text/css")),
        "/app.js" => Some((web_file!("app.js").into(), "text/javascript")),
        "/vendor/codemirror.js" => Some((web_file!("vendor/codemirror.js").into(), "text/javascript")),
        _ => {
            if let Some(name) = path.strip_prefix("/assets/") {
                assets::get(name).map(|a| (Cow::Borrowed(a.bytes), a.mime))
            } else if let Some(rel) = decoded.strip_prefix("/doc/") {
                let file = assets.lock().ok().and_then(|a| a.resolve(rel));
                file.and_then(|f| std::fs::read(&f).ok().map(|b| (Cow::Owned(b), assets::mime_for_path(&f.to_string_lossy()))))
            } else if let Some(abs) = decoded.strip_prefix("/abs/") {
                // 文件中以 Windows 絕對路徑引用的圖片
                let p = PathBuf::from(abs.replace('/', "\\"));
                (DocumentAssets::is_image(&p) && p.is_file())
                    .then(|| std::fs::read(&p).ok())
                    .flatten()
                    .map(|b| (Cow::Owned(b), assets::mime_for_path(abs)))
            } else {
                None
            }
        }
    };

    match found {
        Some((body, mime)) => Response::builder()
            .header(CONTENT_TYPE, mime)
            .header("Cache-Control", "no-store")
            .body(body)
            .unwrap(),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

// ============================== 指令 ==============================

impl App {
    /// 回傳 true 表示要結束程式。
    fn handle_ipc(&mut self, body: &str) -> bool {
        debug_log(&body.chars().take(160).collect::<String>());
        // 拖入檔案的訊息也會經過 wry 的 IPC（附加物件只有 webview2::install_file_drop_hook 拿得到），這裡略過
        if body.starts_with(DROP_MESSAGE_PREFIX) {
            return false;
        }
        let msg: Ipc = match serde_json::from_str(body) {
            Ok(m) => m,
            Err(e) => {
                debug_log(&format!("bad ipc message: {e}"));
                return false;
            }
        };
        match msg {
            Ipc::Ready => self.init_frontend(),
            Ipc::Render { id, text } => {
                let html = render_body(&text);
                self.run_js(&format!("app.rendered({id}, {})", js(&html)));
            }
            Ipc::Dirty { dirty } => {
                self.dirty = dirty;
                self.update_title();
            }
            Ipc::Command { name, arg, text } => return self.command(&name, arg.as_deref(), &text),
            Ipc::Paste { plain } => self.paste(plain),
            Ipc::OpenExternal { url } => open_external(&url),
            Ipc::Settings { patch } => self.patch_settings(patch),
            Ipc::Log { message } => debug_log(&format!("js: {message}")),
            Ipc::MenuKey { key } => open_menu_by_key(&self.window, key.chars().next()),
        }
        false
    }

    fn init_frontend(&mut self) {
        let payload = json!({
            "settings": self.settings,
            "fonts": fonts::installed_families(),
        });
        self.run_js(&format!("app.init({payload})"));
        match self.initial_file.take() {
            Some(path) => self.load(&path),
            None => self.new_document(),
        }
    }

    fn command(&mut self, name: &str, arg: Option<&str>, text: &str) -> bool {
        match name {
            "file.new" => {
                if self.confirm_discard(text) {
                    self.new_document();
                }
            }
            "file.open" => {
                if self.confirm_discard(text)
                    && let Some(path) = self.file_dialog().add_filter(MARKDOWN_FILTER.0, MARKDOWN_FILTER.1).pick_file() {
                        self.load(&path);
                    }
            }
            "file.openPath" => {
                if let Some(path) = arg.map(PathBuf::from)
                    && self.confirm_discard(text) {
                        self.load(&path);
                    }
            }
            "file.save" => {
                self.save(text, false);
            }
            "file.saveAs" => {
                self.save(text, true);
            }
            "file.exportHtml" => self.export_html(text),
            "file.exportPdf" => self.export_pdf(),
            "app.exit" => return self.confirm_discard(text),
            "insert.image" => self.insert_from_dialog(&["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg"], "圖片"),
            "insert.table" => self.insert_from_dialog(&["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"], "表格檔案"),
            "insert.mermaid" => self.insert_from_dialog(&["mmd", "mermaid"], "Mermaid 圖表"),
            "insert.attachment" => self.insert_from_dialog(&[], "所有檔案"),
            "view.toggleTheme" => self.patch_settings(json!({ "dark": !self.settings.dark })),
            "view.toggleEditor" => self.toggle_pane(true),
            "view.togglePreview" => self.toggle_pane(false),
            "help.about" => self.about(),
            other => {
                if let Some(w) = other.strip_prefix("view.width:").and_then(|w| w.parse::<u32>().ok()) {
                    self.patch_settings(json!({ "previewWidth": w }));
                }
            }
        }
        false
    }

    // ---------- 文件 ----------

    fn new_document(&mut self) {
        self.doc_path = None;
        self.assets.lock().unwrap().reset(None);
        self.dirty = false;
        self.run_js(&format!("app.loadDocument({})", json!({ "text": NEW_DOCUMENT, "path": null })));
        self.update_title();
    }

    fn load(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let text = markforge_core::import::spreadsheet::decode_text(
                    &bytes,
                    markforge_core::import::spreadsheet::system_ansi_encoding(),
                );
                self.doc_path = Some(path.to_path_buf());
                self.assets.lock().unwrap().reset(Some(path));
                self.dirty = false;
                self.run_js(&format!(
                    "app.loadDocument({})",
                    json!({ "text": text, "path": path.to_string_lossy() })
                ));
                self.update_title();
            }
            Err(e) => self.warn("開啟失敗", &format!("無法開啟檔案：\n{e}")),
        }
    }

    /// 儲存；成功回傳 true。另存或新文件會先詢問檔名。
    fn save(&mut self, text: &str, save_as: bool) -> bool {
        let path = match (&self.doc_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = self
                    .doc_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or("未命名.md".to_string(), |n| n.to_string_lossy().into_owned());
                match self.file_dialog().add_filter(MARKDOWN_FILTER.0, MARKDOWN_FILTER.1).set_file_name(&name).save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };

        // 先把拖入／貼上的圖片與附件複製到 .md 旁；與資料夾內不同內容的同名檔衝突時會改名，連結跟著改
        let (markdown, failures) = self.assets.lock().unwrap().commit_for_save(text, &path);
        if let Err(e) = std::fs::write(&path, &markdown) {
            self.warn("儲存失敗", &format!("無法儲存檔案：\n{e}"));
            return false;
        }
        self.doc_path = Some(path.clone());
        self.dirty = false;
        let rewritten = (markdown != text).then_some(markdown);
        self.run_js(&format!(
            "app.saved({})",
            json!({ "path": path.to_string_lossy(), "text": rewritten })
        ));
        self.update_title();
        if !failures.is_empty() {
            self.warn("部分檔案未複製", &format!("文件已儲存，但下列附加檔案無法複製到文件資料夾：\n\n{}", failures.join("\n")));
        }
        true
    }

    fn confirm_discard(&mut self, text: &str) -> bool {
        if !self.dirty {
            return true;
        }
        let answer = MessageDialog::new()
            .set_parent(&self.window)
            .set_level(MessageLevel::Warning)
            .set_title("未儲存的變更")
            .set_description("目前文件尚未儲存，是否要先儲存變更？")
            .set_buttons(MessageButtons::YesNoCancel)
            .show();
        match answer {
            MessageDialogResult::Yes => self.save(text, false),
            MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn export_html(&mut self, text: &str) {
        let name = self.doc_stem() + ".html";
        let Some(path) = self.file_dialog().add_filter("HTML 檔案", &["html", "htm"]).set_file_name(&name).save_file() else {
            return;
        };
        let opts = PageOptions {
            dark: self.settings.dark,
            font_family: self.settings.preview_font.clone(),
            font_size: self.settings.preview_font_size,
            width_px: self.settings.preview_width,
            // 匯出檔放在 .md 旁，相對路徑本來就能用
            doc_base: None,
            abs_base: None,
            title: self.doc_stem(),
        };
        let html = page_html(&render_body(text), &opts, AssetMode::Inline);
        if let Err(e) = std::fs::write(&path, html) {
            self.warn("匯出失敗", &format!("無法匯出檔案：\n{e}"));
        }
    }

    /// 前端已把預覽切成適合列印的淺色並畫完；不論結果如何都要呼叫 app.pdfDone() 還原。
    fn export_pdf(&mut self) {
        let name = self.doc_stem() + ".pdf";
        let Some(path) = self.file_dialog().add_filter("PDF 檔案", &["pdf"]).set_file_name(&name).save_file() else {
            self.run_js("app.pdfDone()");
            return;
        };
        let proxy = self.proxy.clone();
        if let Err(e) = webview2::print_to_pdf(&self.webview, &path, move |r| {
            let _ = proxy.send_event(UserEvent::PdfDone(r));
        }) {
            self.run_js("app.pdfDone()");
            self.warn("匯出失敗", &format!("無法匯出 PDF：\n{e}"));
        }
    }

    // ---------- 插入／貼上／拖入 ----------

    fn insert_from_dialog(&mut self, extensions: &[&str], label: &str) {
        let mut dialog = self.file_dialog().set_title(format!("插入{label}"));
        if !extensions.is_empty() {
            dialog = dialog.add_filter(label, extensions);
        }
        if let Some(paths) = dialog.pick_files() {
            self.insert_files(&paths, None);
        }
    }

    fn insert_files(&mut self, paths: &[PathBuf], at: Option<(f64, f64)>) {
        let (snippet, errors) = insert::files(paths, &mut self.assets.lock().unwrap());
        if let Some(s) = snippet {
            self.insert(&s, at);
        }
        if !errors.is_empty() {
            self.warn("插入失敗", &format!("無法讀取下列檔案：\n\n{}", errors.join("\n")));
        }
    }

    fn insert(&self, snippet: &Snippet, at: Option<(f64, f64)>) {
        let payload = json!({ "text": snippet.text, "block": snippet.block, "x": at.map(|a| a.0), "y": at.map(|a| a.1) });
        self.run_js(&format!("app.insert({payload})"));
    }

    fn paste(&mut self, plain: bool) {
        let result = insert::paste(&mut self.assets.lock().unwrap(), plain);
        debug_log(&format!(
            "paste -> {}",
            match &result {
                Paste::Markdown(s) => format!("markdown({} chars)", s.text.len()),
                Paste::Text(t) => format!("text({} chars)", t.len()),
                Paste::Files(s, e) => format!("files({:?}, errors {:?})", s.as_ref().map(|s| s.text.len()), e),
                Paste::Nothing => "nothing".into(),
            }
        ));
        match result {
            Paste::Markdown(s) => self.insert(&s, None),
            Paste::Text(t) => self.insert(&Snippet { text: t, block: false }, None),
            Paste::Files(snippet, errors) => {
                if let Some(s) = snippet {
                    self.insert(&s, None);
                }
                if !errors.is_empty() {
                    self.warn("插入失敗", &format!("無法讀取下列檔案：\n\n{}", errors.join("\n")));
                }
            }
            Paste::Nothing => {}
        }
    }

    /// 單一 .md 檔案 → 開啟（經前端確認未儲存的變更）；其他檔案 → 插入在放開的位置。
    fn files_dropped(&mut self, paths: Vec<PathBuf>, x: f64, y: f64, target: &str) {
        if paths.len() == 1 && insert::is_markdown_file(&paths[0]) {
            self.run_js(&format!("app.command('file.openPath', {})", js(&paths[0].to_string_lossy())));
            return;
        }
        let at = (target == "editor").then_some((x, y));
        self.insert_files(&paths, at);
    }

    // ---------- 設定 ----------

    fn patch_settings(&mut self, patch: serde_json::Value) {
        let mut current = serde_json::to_value(&self.settings).unwrap_or_default();
        if let (Some(obj), Some(p)) = (current.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        let Ok(next) = serde_json::from_value::<Settings>(current) else { return };
        let theme_changed = next.dark != self.settings.dark;
        self.settings = next;
        self.settings.save();
        self.menu.sync(&self.settings);
        if theme_changed {
            let dark = self.settings.dark;
            self.window.set_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
            unsafe {
                let _ = self.menu.menu.set_theme_for_hwnd(window_hwnd(&self.window), menu_theme(dark));
            }
            let _ = self
                .webview
                .set_background_color(if dark { (13, 17, 23, 255) } else { (255, 255, 255, 255) });
        }
        self.run_js(&format!("app.applySettings({})", json!(self.settings)));
    }

    fn toggle_pane(&mut self, editor: bool) {
        let (show_editor, show_preview) = if editor {
            (!self.settings.show_editor, self.settings.show_preview)
        } else {
            (self.settings.show_editor, !self.settings.show_preview)
        };
        if !show_editor && !show_preview {
            self.info("編輯器與預覽不能同時隱藏，至少需保留一個窗格。");
            self.menu.sync(&self.settings);
            return;
        }
        self.patch_settings(json!({ "showEditor": show_editor, "showPreview": show_preview }));
    }

    fn remember_window(&mut self) {
        self.settings.maximized = self.window.is_maximized();
        if !self.settings.maximized {
            let size = self.window.inner_size().to_logical::<u32>(self.window.scale_factor());
            self.settings.window_width = size.width.max(640);
            self.settings.window_height = size.height.max(480);
        }
    }

    // ---------- 小工具 ----------

    fn about(&self) {
        MessageDialog::new()
            .set_parent(&self.window)
            .set_title("關於 MarkForge")
            .set_description(format!(
                "MarkForge Markdown 編輯器 v{}（Rust 版）\n\n\
                 CodeMirror 6 編輯核心、WebView2 即時預覽、comrak（CommonMark + GFM）\n\
                 Mermaid 圖表、Word／Excel 貼上轉 Markdown",
                env!("CARGO_PKG_VERSION")
            ))
            .show();
    }

    fn doc_stem(&self) -> String {
        self.doc_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("未命名".to_string(), |s| s.to_string_lossy().into_owned())
    }

    fn update_title(&self) {
        let name = self
            .doc_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("未命名".to_string(), |n| n.to_string_lossy().into_owned());
        self.window.set_title(&format!("{}{} - {APP_NAME}", if self.dirty { "*" } else { "" }, name));
    }

    fn file_dialog(&self) -> FileDialog {
        let mut d = FileDialog::new().set_parent(&self.window);
        if let Some(dir) = self.doc_path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        d
    }

    fn info(&self, text: &str) {
        MessageDialog::new().set_parent(&self.window).set_title(APP_NAME).set_description(text).show();
    }

    fn warn(&self, title: &str, text: &str) {
        MessageDialog::new()
            .set_parent(&self.window)
            .set_level(MessageLevel::Warning)
            .set_title(title)
            .set_description(text)
            .show();
    }

    fn run_js(&self, script: &str) {
        if let Err(e) = self.webview.evaluate_script(script) {
            eprintln!("evaluate_script: {e}");
        }
    }
}


fn js<T: serde::Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// 以 WM_SYSCOMMAND/SC_KEYMENU 開啟選單列：帶字母時開啟該助憶鍵的選單（Alt+F），
/// 不帶時進入選單列（F10）。效果與焦點在主視窗本身時按下按鍵相同。
fn open_menu_by_key(window: &Window, key: Option<char>) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SC_KEYMENU, WM_SYSCOMMAND};
    let hwnd = HWND(window_hwnd(window) as *mut _);
    let lparam = key.map_or(0, |c| c.to_ascii_lowercase() as isize);
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(SC_KEYMENU as usize), LPARAM(lparam));
    }
}

fn menu_theme(dark: bool) -> MenuTheme {
    if dark { MenuTheme::Dark } else { MenuTheme::Light }
}

fn window_hwnd(window: &Window) -> isize {
    use tao::platform::windows::WindowExtWindows;
    window.hwnd()
}



