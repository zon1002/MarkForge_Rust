//! 依檔案總管的呼叫順序測試預覽處理常式，不需寫入登錄：
//!   Initialize → SetWindow → DoPreview →（6 秒）→ SetRect（縮小）→（2 秒）→ Unload
//! 用法：preview_test <檔案.md>
//! 預覽程式 MarkForgePreviewHost.exe 需在本程式同一個資料夾（cargo build 後的 target\debug 即是）。

use std::time::{Duration, Instant};

use MarkForgePreview::MarkdownPreviewHandler;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::IPreviewHandler;
use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithFile;
use windows::core::{HSTRING, Interface};

fn main() {
    let file = std::env::args().nth(1).expect("用法：preview_test <檔案.md>");
    let file = std::path::absolute(&file).unwrap();

    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::windows::WindowExtWindows;
    let event_loop = EventLoop::new();
    let window = tao::window::WindowBuilder::new()
        .with_title("preview handler test")
        .with_inner_size(tao::dpi::PhysicalSize::new(900u32, 700u32))
        .build(&event_loop)
        .unwrap();
    let hwnd = HWND(window.hwnd() as *mut _);

    let handler: IPreviewHandler = MarkdownPreviewHandler::new().into();
    let init: IInitializeWithFile = handler.cast().unwrap();
    unsafe { init.Initialize(&HSTRING::from(file.as_os_str()), 0).unwrap() };
    let size = window.inner_size();
    let rect = RECT { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 };
    unsafe { handler.SetWindow(hwnd, &rect).unwrap() };
    let t = Instant::now();
    unsafe { handler.DoPreview().unwrap() };
    println!("DoPreview: {} ms", t.elapsed().as_millis());

    let start = Instant::now();
    let mut step = 0;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
        if let Event::NewEvents(StartCause::ResumeTimeReached { .. }) = event {
            let elapsed = start.elapsed();
            if step == 0 && elapsed > Duration::from_secs(6) {
                println!("ready for screenshot 1 (host running: {})", host_running());
                std::fs::write("preview_test_step", "1").ok();
                step = 1;
            } else if step == 1 && elapsed > Duration::from_secs(9) {
                let small = RECT { left: 0, top: 0, right: 600, bottom: 450 };
                unsafe { handler.SetRect(&small).unwrap() };
                println!("SetRect 600x450");
                step = 2;
            } else if step == 2 && elapsed > Duration::from_secs(11) {
                std::fs::write("preview_test_step", "2").ok();
                step = 3;
            } else if step == 3 && elapsed > Duration::from_secs(14) {
                let t = Instant::now();
                unsafe { handler.Unload().unwrap() };
                println!("Unload: {} ms", t.elapsed().as_millis());
                step = 4;
            } else if step == 4 && elapsed > Duration::from_secs(15) {
                println!("host running after Unload: {}", host_running());
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}

fn host_running() -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq MarkForgePreviewHost.exe", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("MarkForgePreviewHost"))
        .unwrap_or(false)
}
