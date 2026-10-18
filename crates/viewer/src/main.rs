//! MarkForge Viewer：唯讀的快速檢視器，適合搭配其他編輯器當「第二螢幕」預覽。
//! 開啟的檔案被任何程式修改存檔後自動重新載入（保留捲動位置）。

#![windows_subsystem = "windows"]

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use markforge_core::attachments::DocumentAssets;
use markforge_core::import::spreadsheet::{decode_text, system_ansi_encoding};
use markforge_core::{assets, render_body};
use markforge_shell::webview2::{self, DROP_MESSAGE_PREFIX};
use markforge_shell::{debug_log, fonts, local_data_dir, open_external, percent_decode, roaming_data_dir};
use notify::{RecursiveMode, Watcher};
use rfd::{FileDialog, MessageDialog, MessageLevel};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::IconExtWindows;
use tao::window::{Icon, Theme, Window, WindowBuilder};
use wry::http::{Request, Response, StatusCode, header::CONTENT_TYPE};
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

const APP_NAME: &str = "MarkForge Viewer";
/// 檔案變更後等這麼久再重新載入：存檔常是好幾次寫入，一次處理就好
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    dark: bool,
    font: String,
    font_size: f32,
    width: u32,
    window_width: u32,
    window_height: u32,
    maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            font: markforge_core::template::DEFAULT_FONT_FAMILY.into(),
            font_size: markforge_core::template::DEFAULT_FONT_SIZE,
            width: markforge_core::template::DEFAULT_WIDTH_PX,
            window_width: 900,
            window_height: 900,
            maximized: false,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        roaming_data_dir().map(|d| d.join("viewer.json"))
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

enum UserEvent {
    Ipc(String),
    FilesDropped(Vec<PathBuf>),
    FileChanged,
    PdfDone(Result<PathBuf, String>),
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum Ipc {
    Ready,
    Open,
    Reload,
    ExportPdf,
    OpenExternal { url: String },
    Settings { patch: serde_json::Value },
    Log { message: String },
}

struct App {
    window: Window,
    webview: WebView,
    proxy: EventLoopProxy<UserEvent>,
    settings: Settings,
    /// 目前文件；`/doc/` 從它的資料夾提供圖片
    current: Arc<Mutex<Option<PathBuf>>>,
    watcher: Option<notify::RecommendedWatcher>,
    reload_at: Option<Instant>,
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

    let current: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let protocol_current = current.clone();
    let ipc_proxy = proxy.clone();
    let mut web_context = WebContext::new(local_data_dir().map(|d| d.join("Viewer").join("WebView2")));
    let webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_custom_protocol("markforge".into(), move |_id, request| serve(&request, &protocol_current))
        .with_url("http://markforge.localhost/viewer.html")
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
    if let Err(e) = webview2::install_file_drop_hook(&webview, move |d| {
        let _ = drop_proxy.send_event(UserEvent::FilesDropped(d.paths));
    }) {
        debug_log(&format!("file drop hook failed: {e}"));
    }

    let mut app = App { window, webview, proxy, settings, current, watcher: None, reload_at: None, initial_file };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = match app.reload_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        };
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if app.reload_at.is_some_and(|at| Instant::now() >= at) {
                    app.reload_at = None;
                    app.render(true);
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.remember_window();
                app.settings.save();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Ipc(body)) => app.handle_ipc(&body),
            Event::UserEvent(UserEvent::FilesDropped(paths)) => {
                if let Some(md) = paths.iter().find(|p| is_markdown(p)) {
                    app.open(md);
                }
            }
            Event::UserEvent(UserEvent::FileChanged) => {
                let at = Instant::now() + RELOAD_DEBOUNCE;
                app.reload_at = Some(at);
                *control_flow = ControlFlow::WaitUntil(at);
            }
            Event::UserEvent(UserEvent::PdfDone(result)) => {
                app.run_js("app.pdfDone()");
                match result {
                    Ok(path) => app.message(MessageLevel::Info, APP_NAME, &format!("已另存為 PDF：\n{}", path.display())),
                    Err(e) => app.message(MessageLevel::Warning, "匯出失敗", &format!("無法匯出 PDF：\n{e}")),
                }
            }
            _ => {}
        }
    });
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}

macro_rules! web_file {
    ($path:literal) => {
        include_bytes!($path).as_slice()
    };
}

fn serve(request: &Request<Vec<u8>>, current: &Arc<Mutex<Option<PathBuf>>>) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path();
    let decoded = percent_decode(path);
    let found: Option<(Cow<'static, [u8]>, &str)> = match path {
        "/" | "/viewer.html" => Some((web_file!("../web/viewer.html").into(), "text/html; charset=utf-8")),
        "/viewer.css" => Some((web_file!("../web/viewer.css").into(), "text/css")),
        "/viewer.js" => Some((web_file!("../web/viewer.js").into(), "text/javascript")),
        // 工具列圖示與 Editor 共用同一份
        _ => {
            if let Some(name) = path.strip_prefix("/assets/") {
                assets::get(name).map(|a| (Cow::Borrowed(a.bytes), a.mime))
            } else if let Some(rel) = decoded.strip_prefix("/doc/") {
                // 只提供文件資料夾之內的檔案
                let folder = current.lock().ok().and_then(|c| c.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)));
                folder
                    .map(|dir| (dir.join(rel.replace('/', "\\")), dir))
                    .filter(|(f, dir)| std::path::absolute(f).is_ok_and(|a| a.starts_with(dir)) && f.is_file())
                    .and_then(|(f, _)| std::fs::read(&f).ok().map(|b| (Cow::Owned(b), assets::mime_for_path(&f.to_string_lossy()))))
            } else if let Some(abs) = decoded.strip_prefix("/abs/") {
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

impl App {
    fn handle_ipc(&mut self, body: &str) {
        if body.starts_with(DROP_MESSAGE_PREFIX) {
            return;
        }
        let msg: Ipc = match serde_json::from_str(body) {
            Ok(m) => m,
            Err(e) => return debug_log(&format!("bad ipc message: {e}")),
        };
        match msg {
            Ipc::Ready => {
                let payload = json!({ "settings": self.settings, "fonts": fonts::installed_families() });
                self.run_js(&format!("app.init({payload})"));
                match self.initial_file.take() {
                    Some(p) => self.open(&p),
                    None => self.run_js("app.empty()"),
                }
            }
            Ipc::Open => {
                let mut dialog = FileDialog::new().set_parent(&self.window).add_filter("Markdown 檔案", &["md", "markdown"]);
                if let Some(dir) = self.current_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.pick_file() {
                    self.open(&path);
                }
            }
            Ipc::Reload => self.render(true),
            Ipc::ExportPdf => self.export_pdf(),
            Ipc::OpenExternal { url } => open_external(&url),
            Ipc::Settings { patch } => self.patch_settings(patch),
            Ipc::Log { message } => debug_log(&format!("js: {message}")),
        }
    }

    fn current_path(&self) -> Option<PathBuf> {
        self.current.lock().ok().and_then(|c| c.clone())
    }

    fn open(&mut self, path: &Path) {
        *self.current.lock().unwrap() = Some(path.to_path_buf());
        self.window.set_title(&format!(
            "{} - {APP_NAME}",
            path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ));
        self.watch(path);
        self.render(false);
    }

    /// 監看文件所在資料夾（直接監看檔案會在「另存＋改名」式的存檔後失效），只理會這個檔案的事件。
    fn watch(&mut self, path: &Path) {
        self.watcher = None;
        let Some(dir) = path.parent() else { return };
        let target = path.to_path_buf();
        let proxy = self.proxy.clone();
        let (tx, rx) = mpsc::channel::<()>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && event.paths.iter().any(|p| p == &target)
            {
                let _ = tx.send(());
            }
        });
        match watcher {
            Ok(mut w) => {
                if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                    // notify 的回呼在背景執行緒；轉成事件迴圈的事件
                    std::thread::spawn(move || {
                        while rx.recv().is_ok() {
                            if proxy.send_event(UserEvent::FileChanged).is_err() {
                                break;
                            }
                        }
                    });
                    self.watcher = Some(w);
                }
            }
            Err(e) => debug_log(&format!("watch failed: {e}")),
        }
    }

    /// 讀檔並顯示；`keep_scroll` 為 true 時（自動重新載入）保留捲動位置。
    fn render(&mut self, keep_scroll: bool) {
        let Some(path) = self.current_path() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let html = render_body(&decode_text(&bytes, system_ansi_encoding()));
                let payload = json!({ "html": html, "path": path.to_string_lossy(), "keepScroll": keep_scroll });
                self.run_js(&format!("app.show({payload})"));
            }
            // 對方可能正在寫入；下一次變更事件會再試
            Err(e) => debug_log(&format!("read failed: {e}")),
        }
    }

    fn export_pdf(&mut self) {
        let name = self
            .current_path()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "未命名".into())
            + ".pdf";
        let Some(path) = FileDialog::new().set_parent(&self.window).add_filter("PDF 檔案", &["pdf"]).set_file_name(&name).save_file()
        else {
            self.run_js("app.pdfDone()");
            return;
        };
        let proxy = self.proxy.clone();
        if let Err(e) = webview2::print_to_pdf(&self.webview, &path, move |r| {
            let _ = proxy.send_event(UserEvent::PdfDone(r));
        }) {
            self.run_js("app.pdfDone()");
            self.message(MessageLevel::Warning, "匯出失敗", &format!("無法匯出 PDF：\n{e}"));
        }
    }

    fn patch_settings(&mut self, patch: serde_json::Value) {
        let mut current = serde_json::to_value(&self.settings).unwrap_or_default();
        if let (Some(obj), Some(p)) = (current.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        let Ok(next) = serde_json::from_value::<Settings>(current) else { return };
        if next.dark != self.settings.dark {
            self.window.set_theme(Some(if next.dark { Theme::Dark } else { Theme::Light }));
            let _ = self
                .webview
                .set_background_color(if next.dark { (13, 17, 23, 255) } else { (255, 255, 255, 255) });
        }
        self.settings = next;
        self.settings.save();
        self.run_js(&format!("app.applySettings({})", json!(self.settings)));
    }

    fn remember_window(&mut self) {
        self.settings.maximized = self.window.is_maximized();
        if !self.settings.maximized {
            let size = self.window.inner_size().to_logical::<u32>(self.window.scale_factor());
            self.settings.window_width = size.width.max(480);
            self.settings.window_height = size.height.max(360);
        }
    }

    fn message(&self, level: MessageLevel, title: &str, text: &str) {
        MessageDialog::new().set_parent(&self.window).set_level(level).set_title(title).set_description(text).show();
    }

    fn run_js(&self, script: &str) {
        if let Err(e) = self.webview.evaluate_script(script) {
            debug_log(&format!("evaluate_script: {e}"));
        }
    }
}
