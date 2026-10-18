//! Entry point for RustBrowser.
//!
//! Runs the windowed browser by default. `--screenshot <url> <out.png> [width]` renders one
//! page to a PNG without opening a window, and `--no-js` disables script execution.

mod browser;
mod css;
mod dom;
mod font;
mod html;
mod js;
mod jsdom;
mod layout;
mod net;
mod page;
mod paint;
mod style;

use browser::{Browser, parse_input};
use font::FontStore;
use page::Page;
use paint::{Canvas, render};
use url::Url;

fn screenshot(target: &str, out: &str, width: usize, fonts: &FontStore) -> Result<(), String> {
    let url = parse_input(target).ok_or("Invalid URL")?;
    let mut page = Page::load(url);
    page.layout(width as f32, 800.0, fonts);
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(400) {
        page.tick();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = page.take_requests();
    if page.needs_layout() {
        page.layout(width as f32, 800.0, fonts);
    }
    let height = (page.height.ceil() as usize).clamp(1, 20000);
    let mut buf = vec![0u32; width * height];
    let mut canvas = Canvas::new(&mut buf, width, height);
    canvas.fill(page.bg);
    render(&page.cmds, &mut canvas, 0.0, 0.0, 1.0, fonts);
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let data: Vec<u8> = buf
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    println!("{}: {}x{} -> {}", page.url, width, height, out);
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--no-js") {
        args.remove(i);
        page::JS_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let fonts = match FontStore::load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    if args.first().map(String::as_str) == Some("--screenshot") {
        if args.len() < 3 {
            eprintln!("usage: rustbrowser --screenshot <url> <out.png> [width]");
            std::process::exit(2);
        }
        let width = args.get(3).and_then(|w| w.parse().ok()).unwrap_or(1024);
        if let Err(e) = screenshot(&args[1], &args[2], width, &fonts) {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        return;
    }
    let start = args
        .first()
        .and_then(|a| parse_input(a))
        .unwrap_or_else(|| Url::parse("about:home").unwrap());
    match Browser::new(start, fonts) {
        Ok(mut b) => b.run(),
        Err(e) => {
            eprintln!("Could not open a window: {}", e);
            std::process::exit(1);
        }
    }
}
