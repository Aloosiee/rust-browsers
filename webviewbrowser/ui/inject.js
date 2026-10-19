// Runs in the main frame of every page before the page's own scripts.
//
// It forwards browser shortcuts, reports URL changes made by single-page apps, reports the
// favicon and fullscreen state, and implements find in page. Everything a page could forge
// here is treated as untrusted by the browser.
(() => {
  'use strict';
  if (window.__browserInjected) return;
  Object.defineProperty(window, '__browserInjected', { value: true });

  const send = (message) => {
    try {
      window.ipc.postMessage(JSON.stringify({ ...message, href: location.href }));
    } catch (_) {
      // The bridge is missing in some sandboxed documents; nothing to report to.
    }
  };

  /* SHORTCUTS */

  window.addEventListener('keydown', (e) => {
    const hit = BrowserShortcuts.match(e);
    if (!hit || !hit.reserved) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    send({ kind: 'shortcut', action: hit.action });
  }, true);

  window.addEventListener('keydown', (e) => {
    const hit = BrowserShortcuts.match(e);
    if (!hit || hit.reserved || e.defaultPrevented) return;
    e.preventDefault();
    send({ kind: 'shortcut', action: hit.action });
  });

  let lastWheelZoom = 0;
  window.addEventListener('wheel', (e) => {
    if (!e.ctrlKey || e.defaultPrevented) return;
    e.preventDefault();
    const now = Date.now();
    if (now - lastWheelZoom < 120) return;
    lastWheelZoom = now;
    send({ kind: 'shortcut', action: e.deltaY < 0 ? 'zoomIn' : 'zoomOut' });
  }, { passive: false });

  for (const method of ['pushState', 'replaceState']) {
    const original = history[method];
    history[method] = function (...args) {
      const result = original.apply(this, args);
      send({ kind: 'location' });
      return result;
    };
  }
  window.addEventListener('popstate', () => send({ kind: 'location' }));
  window.addEventListener('hashchange', () => send({ kind: 'location' }));

  const reportIcon = () => {
    const link = document.querySelector('link[rel~="icon" i], link[rel="shortcut icon" i], link[rel="apple-touch-icon" i]');
    const fallback = /^https?:$/.test(location.protocol) ? location.origin + '/favicon.ico' : '';
    const url = link && link.href ? link.href : fallback;
    if (url) send({ kind: 'favicon', url });
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', reportIcon, { once: true });
  } else {
    reportIcon();
  }

  document.addEventListener('fullscreenchange', () => {
    send({ kind: 'fullscreen', on: !!document.fullscreenElement });
  });

  // Engines ignore window.close() in tabs the browser created for popups, so a popup that
  // closes itself (sign-in flows do) asks the browser instead. `opener` is checked at call
  // time because WebKit sets it after this script runs.
  const nativeClose = window.close.bind(window);
  window.close = function () {
    if (window.opener) send({ kind: 'close' });
    else nativeClose();
  };

  /* FIND IN PAGE */

  const find = (() => {
    const supportsHighlights = typeof CSS !== 'undefined' && CSS.highlights && typeof Highlight === 'function';
    let ranges = [];
    let index = -1;
    let lastQuery = '';
    let styled = false;
    let allHighlight = null;
    let currentHighlight = null;

    const skip = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEXTAREA', 'TEMPLATE']);

    function collect(query) {
      ranges = [];
      const root = document.body || document.documentElement;
      if (!query || !root) return;
      const needle = query.toLowerCase();
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          const parent = node.parentElement;
          if (!parent || skip.has(parent.tagName)) return NodeFilter.FILTER_REJECT;
          return NodeFilter.FILTER_ACCEPT;
        },
      });
      while (walker.nextNode()) {
        const node = walker.currentNode;
        const text = node.data.toLowerCase();
        if (text.length !== node.data.length) continue;
        let at = text.indexOf(needle);
        if (at < 0 || node.parentElement.getClientRects().length === 0) continue;
        while (at >= 0) {
          const range = new Range();
          range.setStart(node, at);
          range.setEnd(node, at + needle.length);
          ranges.push(range);
          if (ranges.length >= 10000) return;
          at = text.indexOf(needle, at + needle.length);
        }
      }
    }

    function paint() {
      const current = ranges[index];
      if (supportsHighlights) {
        if (!styled) {
          const style = document.createElement('style');
          style.textContent =
            '::highlight(browser-find){background:#f8e45c;color:#000}' +
            '::highlight(browser-find-current){background:#ff9632;color:#000}';
          (document.head || document.documentElement).appendChild(style);
          styled = true;
        }
        if (!allHighlight) {
          allHighlight = new Highlight();
          currentHighlight = new Highlight();
          CSS.highlights.set('browser-find', allHighlight);
          CSS.highlights.set('browser-find-current', currentHighlight);
        }
        // Updating registered highlights in place (instead of replacing them) makes WebKit
        // repaint the ranges that were removed.
        for (const r of [...allHighlight]) allHighlight.delete(r);
        for (const r of [...currentHighlight]) currentHighlight.delete(r);
        for (const r of ranges) allHighlight.add(r);
        if (current) currentHighlight.add(current);
      } else if (current) {
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(current);
      }
      if (current) {
        const element = current.startContainer.parentElement;
        const rect = current.getBoundingClientRect();
        if (rect.top < 0 || rect.bottom > window.innerHeight || rect.left < 0 || rect.right > window.innerWidth) {
          element.scrollIntoView({ block: 'center', inline: 'nearest' });
        }
      }
    }

    function clear() {
      ranges = [];
      index = -1;
      lastQuery = '';
      if (allHighlight) {
        for (const r of [...allHighlight]) allHighlight.delete(r);
        for (const r of [...currentHighlight]) currentHighlight.delete(r);
      }
    }

    return (query, backwards) => {
      if (query === null || query === '') {
        clear();
        send({ kind: 'findResult', current: 0, total: 0 });
        return;
      }
      const same = query === lastQuery;
      collect(query);
      lastQuery = query;
      if (ranges.length === 0) {
        index = -1;
      } else if (!same) {
        index = backwards ? ranges.length - 1 : 0;
      } else {
        index = (Math.max(index, 0) + (backwards ? -1 : 1) + ranges.length) % ranges.length;
      }
      paint();
      send({ kind: 'findResult', current: index + 1, total: ranges.length });
    };
  })();

  Object.defineProperty(window, '__browserFind', { value: find });
})();
