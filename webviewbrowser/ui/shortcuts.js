// Keyboard shortcuts shared by the toolbar and every web page.
//
// "Reserved" shortcuts always belong to the browser, like in Firefox. Other shortcuts only
// fire if the page did not handle the key itself (for example a web app with its own Ctrl+F).
const BrowserShortcuts = (() => {
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  const isLinux = /Linux/.test(navigator.platform);
  const table = [
    { key: 't', primary: true, action: 'newTab', reserved: true },
    { key: 't', primary: true, shift: true, action: 'reopenClosedTab', reserved: true },
    { key: 'w', primary: true, action: 'closeTab', reserved: true },
    { key: 'F4', primary: true, action: 'closeTab', reserved: true },
    { key: 'Tab', ctrl: true, action: 'nextTab', reserved: true },
    { key: 'Tab', ctrl: true, shift: true, action: 'previousTab', reserved: true },
    { key: 'PageDown', ctrl: true, action: 'nextTab', reserved: true },
    { key: 'PageUp', ctrl: true, action: 'previousTab', reserved: true },
    { key: 'n', primary: true, action: 'newWindow', reserved: true },
    { key: 'p', primary: true, shift: true, action: 'newPrivateWindow', reserved: true },
    { key: 'w', primary: true, shift: true, action: 'closeWindow', reserved: true },
    { key: 'q', primary: true, action: 'quit', reserved: true, onlyOn: (mac, linux) => mac || linux },
    { key: 'q', primary: true, shift: true, action: 'quit', reserved: true },
    { key: 'l', primary: true, action: 'focusAddressBar', reserved: true },
    { key: 'k', primary: true, action: 'focusAddressBar' },
    { key: 'd', alt: true, action: 'focusAddressBar', reserved: true },
    { key: 'F6', action: 'focusAddressBar', reserved: true },
    { key: 'r', primary: true, action: 'reload' },
    { key: 'r', primary: true, shift: true, action: 'reload' },
    { key: 'F5', action: 'reload' },
    { key: 'F5', primary: true, action: 'reload' },
    { key: 'ArrowLeft', alt: true, action: 'back' },
    { key: 'ArrowRight', alt: true, action: 'forward' },
    { key: '[', meta: true, action: 'back', onlyOn: (mac) => mac },
    { key: ']', meta: true, action: 'forward', onlyOn: (mac) => mac },
    { key: 'Home', alt: true, action: 'home' },
    { key: 'Escape', action: 'stop' },
    { key: 'd', primary: true, action: 'bookmarkPage' },
    { key: 'b', primary: true, shift: true, action: 'toggleBookmarksBar' },
    { key: 'o', primary: true, shift: true, action: 'showBookmarks' },
    { key: 'h', primary: true, action: 'showHistory' },
    { key: 'j', primary: true, action: 'showDownloads' },
    { key: 'y', primary: true, shift: true, action: 'showDownloads' },
    { key: 'f', primary: true, action: 'find' },
    { key: 'F3', action: 'findNext' },
    { key: 'F3', shift: true, action: 'findPrevious' },
    { key: 'g', primary: true, action: 'findNext' },
    { key: 'g', primary: true, shift: true, action: 'findPrevious' },
    { key: 'p', primary: true, action: 'print' },
    { key: '=', primary: true, anyShift: true, action: 'zoomIn' },
    { key: '+', primary: true, anyShift: true, action: 'zoomIn' },
    { key: '-', primary: true, anyShift: true, action: 'zoomOut' },
    { key: '_', primary: true, anyShift: true, action: 'zoomOut' },
    { key: '0', primary: true, action: 'zoomReset' },
    { key: 'F11', action: 'fullscreen', reserved: true },
    { key: 'F12', action: 'devTools', reserved: true },
    { key: 'i', primary: true, shift: true, action: 'devTools', reserved: true },
    { key: ',', primary: true, action: 'showSettings', onlyOn: (mac) => mac },
  ];
  for (let n = 1; n <= 8; n++) {
    table.push({ key: String(n), primary: !isLinux, alt: isLinux, action: 'selectTab' + n, reserved: true });
  }
  table.push({ key: '9', primary: !isLinux, alt: isLinux, action: 'selectLastTab', reserved: true });

  function match(e) {
    if (e.isComposing) return null;
    const primary = isMac ? e.metaKey : e.ctrlKey;
    const key = e.key && e.key.length === 1 ? e.key.toLowerCase() : e.key;
    for (const s of table) {
      if (s.onlyOn && !s.onlyOn(isMac, isLinux)) continue;
      if (s.key.toLowerCase() !== key && s.key !== e.key) continue;
      if (!!s.primary !== primary && !s.ctrl) continue;
      if (s.ctrl && !e.ctrlKey) continue;
      if (!s.ctrl && !s.primary && (e.ctrlKey || (isMac && e.metaKey))) continue;
      if (!!s.alt !== e.altKey) continue;
      if (!s.anyShift && !!s.shift !== e.shiftKey) continue;
      if (!!s.meta && !e.metaKey) continue;
      return { action: s.action, reserved: !!s.reserved };
    }
    return null;
  }

  return { match };
})();
