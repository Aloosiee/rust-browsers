# webviewbrowser

A tabbed desktop browser that uses the web engine already on your computer: Microsoft Edge WebView2 (Chromium) on Windows and WebKitGTK on Linux. Sites render and run the same as they do in Edge or GNOME Web, so this is the browser in the repo for day-to-day use. The Rust code adds the parts a browser needs on top of the engine: tabs, windows, history, bookmarks, downloads, sessions and settings.

![webviewbrowser with two tabs and the bookmarks toolbar](../docs/webviewbrowser.png)

## Features

The features and keyboard shortcuts follow Firefox.

- Tabs: open, close, reorder by dragging, close with a middle click, reopen closed tabs (Ctrl+Shift+T), switch with Ctrl+Tab, Ctrl+PageUp/PageDown or the number keys.
- Windows: new windows (Ctrl+N) and private windows (Ctrl+Shift+P). Private windows keep cookies and site data in a throwaway folder that is deleted when the last private window closes, and record no history.
- Address bar: takes URLs, host names, local file paths or search terms. Suggestions come from bookmarks and history as you type, ranked by how often and how recently you visited. Start with `?` to force a search. Alt+Enter opens the result in a new tab.
- Search engines: DuckDuckGo, Google, Bing, Startpage, Ecosia, or a custom URL with `%s` for the query.
- Bookmarks: star a page (Ctrl+D), a bookmarks toolbar (Ctrl+Shift+B) and a page to search, edit and reorder them (Ctrl+Shift+O).
- History page (Ctrl+H) with search, per-day grouping, and clearing by time range.
- Downloads: progress badge on the toolbar, a downloads page (Ctrl+J) with open and show-in-folder, and a choice of download folder.
- Find in page (Ctrl+F) with match count, next and previous.
- Zoom (Ctrl+plus, Ctrl+minus, Ctrl+0, Ctrl+scroll) in Firefox's zoom steps, remembered per site, with a default zoom setting.
- Restore the previous session on start. Restored tabs load when you first switch to them.
- Popups opened by `window.open` become tabs that stay linked to their opener, so sign-in popups that post a message back and close themselves work.
- Links to other apps (`mailto:`, `zoommtg:` and so on) ask before opening the app.
- Camera, microphone and location requests ask for permission (Linux; on Windows WebView2 shows its own prompt).
- Full screen (F11), print (Ctrl+P), and the engine's developer tools (F12).
- Light and dark themes that follow the system.
- Browser extensions on Windows: put unpacked extensions in the profile's `extensions` folder. WebView2 supports a subset of the Chrome extension API.

## Build and run

Install Rust from [rustup.rs](https://rustup.rs).

On Windows, install the Visual Studio C++ build tools. Windows 10 and 11 already include the WebView2 runtime. Then:

```
cd webviewbrowser
cargo run --release
```

On Linux, install the WebKitGTK and GTK development packages first.

Debian and Ubuntu:

```
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libxdo-dev build-essential
```

Fedora (and Bazzite or Silverblue inside a Fedora toolbox or distrobox):

```
sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel libxdo-devel gcc
```

Arch:

```
sudo pacman -S webkit2gtk-4.1 gtk3 libsoup3 xdotool base-devel
```

Then build and run as above. The first release build takes a few minutes.

### Command line

```
webviewbrowser [--private] [--profile DIR] [URL...]
```

- `URL...` opens each address in a tab. Anything the address bar accepts works, including search terms in quotes.
- `--private` starts with a private window.
- `--profile DIR` keeps all data in `DIR` instead of the default profile.

## Where data is stored

The profile lives in the user's local data folder:

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\rust-browsers\webviewbrowser` |
| Linux | `~/.local/share/rust-browsers/webviewbrowser` |

It holds `browser.sqlite` (history, bookmarks, downloads and per-site zoom), `settings.json`, `session.json`, the engine's cookies and site data in `webdata`, and `extensions`. Settings → Privacy can clear cookies and site data, history, or zoom levels.

## How it works

```
src/
  main.rs         command line, event loop
  app.rs          windows, tabs, and handling of every message and event
  window.rs       one browser window: toolbar webview, tab webviews, layout
  tab.rs          building a tab's webview and wiring its callbacks
  ipc.rs          message types from the toolbar and from pages
  actions.rs      named commands shared by shortcuts and the menu
  protocol.rs     the browser: scheme that serves ui/
  urlbar.rs       turning address bar input into a URL or a search
  store.rs        SQLite storage for history, bookmarks, downloads and zoom
  settings.rs     settings.json
  session.rs      session.json
  menu.rs         the native ☰ menu
  permissions.rs  camera, microphone and location prompts on Linux
ui/
  chrome.*        the toolbar: tabs, address bar, find bar, bookmarks bar
  inject.js       runs in every page: shortcuts, find in page, page state reports
  shortcuts.js    the keyboard shortcut table, shared by toolbar and pages
  pages/          browser://newtab, history, bookmarks, downloads, settings
```

Each window holds one webview for the toolbar and one per tab. The toolbar is HTML served from the `browser:` scheme and talks to Rust through the webview's message channel. Rust sends it the state to draw, and it sends back what the user did.

Each page gets `inject.js` before its own scripts. It catches browser shortcuts (keys like Ctrl+T always go to the browser; keys like Ctrl+F go to the browser only if the page didn't handle them), implements find in page, and reports URL changes from single-page apps, the favicon and full screen state.

Pages can post messages too, so Rust treats every page message as untrusted: web pages can only trigger harmless actions, and only `browser:` pages may call the storage API. Web pages cannot navigate to `browser:` URLs, and `browser:` pages cannot be framed.

## Limitations

- No password manager, sync, reader view or picture-in-picture controls.
- Extensions work only on Windows.
- Keyboard shortcuts typed while focus is inside an iframe go to the iframe, not the browser.
- macOS should build (wry and tao support it), but it has not been tested and the menu shortcut labels are not aligned there.
- Each tab is a full webview, so dozens of open tabs use more memory than in Firefox, which shares processes more aggressively.
