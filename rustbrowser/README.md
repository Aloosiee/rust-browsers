# RustBrowser

A web browser with its own rendering engine, written in Rust. It parses HTML and CSS, computes styles, lays out pages, paints them into a window, and runs JavaScript against a live DOM.

The engine (parsing, cascade, layout, painting, DOM bindings) is written from scratch. Crates handle JavaScript execution (Boa), the window and framebuffer (minifb), glyph rasterization (fontdue), HTTP and TLS (ureq), URL parsing (url) and PNG output (png).

## Build and run

```
cargo run --release
cargo run --release -- https://example.com
cargo run --release -- path/to/page.html
cargo run --release -- --no-js path/to/page.html
```

Render a page to a PNG without opening a window (timers get 400 ms to run first):

```
cargo run --release -- --screenshot https://example.com out.png 1024
```

The browser looks for DejaVu, Liberation, Noto, Arial/Times/Courier (macOS and Windows) fonts in the usual system locations. On Linux, `fonts-dejavu` or `fonts-liberation` covers it.

If you are behind a proxy that re-signs TLS, point `SSL_CERT_FILE` at its CA bundle.

## Using it

The start page (`about:home`) lists sites that render well. Type a URL, a local path, or search terms (sent to DuckDuckGo Lite) in the address bar. Click a text field on a page to type into it; Enter submits its form.

| Keys | Action |
|---|---|
| Ctrl+L, F6 | Focus the address bar |
| Enter / Esc | Go / cancel editing |
| Alt+Left, Backspace | Back |
| Alt+Right | Forward |
| F5, Ctrl+R | Reload |
| Space, PgUp, PgDn, arrows, Home, End, mouse wheel | Scroll |
| Ctrl+Plus, Ctrl+Minus, Ctrl+0, Ctrl+wheel | Zoom |

Run `cargo test` for the unit tests (parser, cascade, layout, DOM and JavaScript runtime). `test-pages/js.html` exercises the JavaScript support and reports pass/fail for each feature. `test-pages/stress.html` covers HTML and CSS edge cases.

## JavaScript

Scripts run in Boa, a JavaScript engine written in Rust. The DOM lives in an arena (`jsdom.rs`) shared between the engine and the scripts; any change marks the page for restyle and layout on the next frame.

`js.rs` binds the core DOM in Rust: node traversal, `appendChild`/`insertBefore`/`removeChild`, attributes, `innerHTML`/`outerHTML`, `textContent`, `querySelector(All)` (using the engine's own selector matcher), `getBoundingClientRect` (from the last layout), form values, timers, network and storage. `prelude.js` builds the rest of the web platform on top in JavaScript: events with bubbling, `preventDefault` and inline `on*` handlers, `style` and `dataset` (as proxies over attributes), `classList`, `setTimeout`/`setInterval`/`requestAnimationFrame`, `fetch`, `XMLHttpRequest`, `localStorage`, `location`, `history.pushState`, `URL`, `URLSearchParams`, `matchMedia`, `alert`/`confirm`, `console`, and stubs for observers and custom elements so modern scripts don't crash on load.

The browser sends real click, key, input, change, submit, focus and load events. Clicking links, checkboxes, radio buttons, selects and submit buttons does the default action unless a handler cancels it. GET forms submit; POST does not.

## How it works

| File | Stage |
|---|---|
| `html.rs` | Tokenizer and tree builder. Handles raw-text elements, entities, void elements, implied end tags (`p`, `li`, `td`, `tr`, `option`, headings) and adds missing `html`/`body`. |
| `dom.rs` | Node tree handed to the style system, with node ids that map back to the arena. |
| `jsdom.rs` | Mutable DOM arena, serialization, fragment parsing, selector queries. |
| `js.rs`, `prelude.js` | JavaScript runtime and DOM bindings. |
| `css.rs` | Stylesheet parser: rules, `@media` (width, print, color scheme), `@supports`, selectors with type, class, ID, attribute and `:root`/`:link`, descendant and child combinators, specificity, values, colors (`#hex`, `rgb()`, `hsl()`, names). |
| `style.rs` | Selector matching (indexed by ID, class and tag), cascade by origin, importance, specificity and order, inheritance, `em`/`rem`/`%`/`vw` resolution, shorthands, legacy attributes (`bgcolor`, `align`, `font`, `border`, `cellpadding`). |
| `layout.rs` | Box tree with anonymous inline boxes. Block layout with auto margins, `max-width`, `box-sizing` and margin collapsing (siblings and parent/child). Inline layout with word wrapping, `text-align`, `line-height`, `white-space`, `text-transform`. Table layout sized from min/max content widths, with `colspan` and vertical alignment. List markers. |
| `paint.rs` | Display list, hit-test regions and a software rasterizer for rectangles, glyphs and markers. |
| `font.rs` | Font discovery, per-character fallback, glyph and advance caches. |
| `page.rs` | Loads a document, its stylesheets and scripts, runs style and layout, dispatches events and default actions. |
| `net.rs` | `http`, `https`, `file` (with directory listings) and `about:` pages. |
| `browser.rs` | Window, toolbar, address bar, history with saved scroll positions, scrolling, zoom, alert dialogs. |

## Not supported

Images (shown as their alt text), flexbox and grid (laid out as blocks), floats, positioning, `inline-block` sizing, inline padding and borders, POST forms, cookies sent to servers, ES modules, `<style>` elements added by scripts after load, and selectors with `+`, `~` or pseudo-classes other than `:root` and `:link`. Pages built on large frameworks usually run their scripts without errors but still look plain, because of the layout limits above.

For a browser that renders modern sites, see [`servobrowser`](../servobrowser).
