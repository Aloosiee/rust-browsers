# rust-browsers

Two web browsers written in Rust, built to compare what it takes to write a browser engine yourself with what you get by embedding a production one.

| | [`rustbrowser`](rustbrowser) | [`servobrowser`](servobrowser) |
|---|---|---|
| Engine | Written from scratch in this repo | [Servo](https://servo.org) |
| HTML and CSS | Own parser, cascade, block, inline and table layout | Servo's parser, Stylo (also used in Firefox), full CSS layout |
| JavaScript | Boa, with DOM bindings written here | SpiderMonkey (Firefox's engine) |
| Rendering | Software rasterizer into a minifb window | WebRender on OpenGL, with an egui toolbar |
| Real-world sites | Simple and static pages | Many modern sites |
| First build | About 5 minutes | 25+ minutes, about 3 GB |
| Code | About 8,500 lines of Rust and 600 of JavaScript | About 1,200 lines of Rust |

Both have an address bar that accepts URLs, local paths or search terms, back and forward history, reload, zoom, and a headless `--screenshot` mode for rendering a page to a PNG.

Both pass the same JavaScript test page ([`rustbrowser/test-pages/js.html`](rustbrowser/test-pages/js.html)):

| rustbrowser | servobrowser |
|---|---|
| ![rustbrowser rendering the JavaScript test page](docs/rustbrowser.png) | ![servobrowser rendering the JavaScript test page](docs/servobrowser.png) |

## Quick start

Install Rust from [rustup.rs](https://rustup.rs), then:

```
cd rustbrowser
cargo run --release
```

```
cd servobrowser
cargo run --release
```

The Servo browser needs system libraries first; see [its README](servobrowser/README.md#build). On Windows, install the Visual Studio C++ build tools before running either.

## Repository layout

```
rustbrowser/    from-scratch engine and browser
  src/          parser, style, layout, paint, JavaScript runtime, window
  test-pages/   HTML pages for checking rendering and JavaScript by eye
servobrowser/   desktop shell around Servo
  src/          window, tabs, input routing, dialogs, browser: pages
docs/           screenshots
.github/        CI: formatting, build and unit tests
```

## Development

Each crate is independent, with its own `Cargo.lock`. Run `cargo fmt` before committing; CI checks formatting for both crates and runs `cargo test` for `rustbrowser`. CI does not build `servobrowser` because Servo takes too long to compile there, so build it locally after changing it.

## License

MIT, see [LICENSE](LICENSE). Servo is MPL-2.0 and Boa is MIT/Unlicense; both are used as unmodified dependencies.
