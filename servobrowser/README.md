# Servo Browser

A desktop web browser written in Rust on top of [Servo](https://servo.org), the engine Mozilla started. Servo uses two components that also ship in Firefox: SpiderMonkey runs JavaScript and Stylo computes CSS. WebRender draws the page with the GPU.

Mozilla doesn't offer Gecko, Firefox's own engine, as an embeddable library on desktop, so Servo is the closest you can get to building on Firefox in Rust.

This project is the browser around the engine: window, tabs, toolbar, address bar, input handling, dialogs and a start page. Servo does the parsing, styling, layout, scripting and rendering.

## Build

Servo is a large dependency. The first build compiles about 650 crates and needs around 3 GB of disk; it took 25 minutes on a 2-core machine and is faster with more cores. Later builds of this crate take seconds.

Linux (Debian/Ubuntu):

```
sudo apt install build-essential clang cmake pkg-config python3 libfreetype6-dev libfontconfig-dev \
  libharfbuzz-dev libgl1-mesa-dri libgles2-mesa-dev libegl1-mesa-dev libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libxkbcommon-dev libwayland-dev libdbus-1-dev libudev-dev libunwind-dev
cargo build --release
```

macOS needs Xcode; Windows needs the Visual Studio C++ build tools. The Servo Book's [build page](https://book.servo.org/building/building.html) lists platform dependencies in detail.

## Run

```
cargo run --release
cargo run --release -- https://servo.org
cargo run --release -- path/to/page.html
```

Render a page to a PNG without a window (uses Servo's software renderer):

```
cargo run --release -- --screenshot https://servo.org out.png 1200 800
```

## Using it

The address bar takes URLs, local paths or search terms (sent to DuckDuckGo). The start page lives at `browser:home`, served by a custom protocol handler.

| Keys | Action |
|---|---|
| Ctrl+L, Alt+D, F6 | Focus the address bar |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Tab, Ctrl+Shift+Tab | Next / previous tab |
| Alt+Left / Alt+Right, mouse back/forward buttons | Back / forward |
| Ctrl+R, F5 | Reload |
| Ctrl+Plus, Ctrl+Minus, Ctrl+0 | Zoom |
| Ctrl+Q | Quit |

On macOS, Cmd replaces Ctrl.

`alert`, `confirm` and `prompt` open dialogs labelled with the page's host. `<select>` menus open a picker. Links with `target=_blank` and `window.open` open new tabs. Middle-click a tab to close it.

## How it's built

| File | Purpose |
|---|---|
| `src/main.rs` | winit event loop, the `Browser` (tabs, toolbar, dialogs, input routing), Servo's `WebViewDelegate` callbacks, and headless screenshot mode. |
| `src/keys.rs` | Converts winit keyboard events to the W3C key and code values Servo expects. |
| `src/protocol.rs` | `browser:` URL scheme for built-in pages. |
| `src/home.html` | Start page. |

The window owns one OpenGL context. Every tab's `WebView` renders into a shared offscreen framebuffer; each frame, egui draws the tab strip and toolbar and blits the active tab's framebuffer into the area below them. Mouse events below the toolbar and keyboard events (unless the address bar or a dialog has focus) go to the active `WebView`.

## Limits

No bookmarks, downloads, password storage, extensions or settings page yet. Servo's own web compatibility is improving monthly but is behind Firefox and Chrome; complex web apps may misrender. Servo publishes breaking API changes in its monthly releases, so moving past `servo = "0.7.0"` may need code changes.

## License

MIT, see [LICENSE](../LICENSE). Servo itself is licensed under MPL-2.0.
