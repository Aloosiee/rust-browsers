//! A tabbed desktop web browser built on the operating system's web engine: Microsoft Edge
//! WebView2 (Chromium) on Windows, WebKitGTK on Linux and WKWebView on macOS.
//!
//! The toolbar and the `browser:` pages are HTML in `ui/`; Rust owns windows, tabs, storage
//! and every decision that needs privileges.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod app;
mod event;
mod ipc;
mod menu;
mod paths;
#[cfg(target_os = "linux")]
mod permissions;
mod protocol;
mod session;
mod settings;
mod store;
mod tab;
mod urlbar;
mod window;

use app::{App, Startup};
use event::UserEvent;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

const USAGE: &str = "usage: webviewbrowser [--private] [--profile DIR] [URL...]";
const SESSION_SAVE_INTERVAL: Duration = Duration::from_secs(5);

struct Args {
    startup: Startup,
    profile: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut urls = Vec::new();
    let mut private = false;
    let mut profile = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--private" => private = true,
            "--profile" => profile = Some(PathBuf::from(args.next().ok_or("--profile needs a folder")?)),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            _ => urls.push(arg),
        }
    }
    Ok(Args {
        startup: Startup { urls, private },
        profile,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // WebKitGTK's DMA-BUF renderer shows blank pages on some NVIDIA and virtual GPU setups.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event.id().0.clone()));
    }));

    let profile = paths::Profile::locate(args.profile);
    let app = match App::new(proxy, profile) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let mut startup = Some(args.startup);
    let mut next_save = Instant::now() + SESSION_SAVE_INTERVAL;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_save);
        let mut app = app.borrow_mut();
        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Some(s) = startup.take() {
                    app.start(target, s);
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                app.save_session_if_dirty();
                next_save = Instant::now() + SESSION_SAVE_INTERVAL;
                *control_flow = ControlFlow::WaitUntil(next_save);
            }
            Event::WindowEvent { window_id, event, .. } => match event {
                WindowEvent::CloseRequested => app.close_window(window_id),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => app.layout(window_id),
                _ => {}
            },
            Event::UserEvent(e) => app.handle_user_event(target, e),
            _ => {}
        }
        if app.quit_requested || (startup.is_none() && !app.has_windows()) {
            app.shutdown();
            *control_flow = ControlFlow::Exit;
        }
    });
}
