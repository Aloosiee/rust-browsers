// Shared helpers for the browser's own pages. `browser.call` sends a request to Rust and
// resolves with the reply; Rust answers through `window.__browserResolve`.
'use strict';

const browser = (() => {
  let nextId = 1;
  const pending = new Map();

  Object.defineProperty(window, '__browserResolve', {
    value(id, ok, data) {
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(data);
      else p.reject(new Error(data));
    },
  });

  function call(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ kind: 'request', id, method, params }));
    });
  }

  function onChange(topic, handler) {
    window.addEventListener('browser:changed', (e) => {
      if (e.detail === topic) handler();
    });
  }

  return { call, onChange };
})();

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children) {
    if (c === null || c === undefined) continue;
    node.append(c instanceof Node ? c : String(c));
  }
  return node;
}

function favicon(url) {
  const img = el('img', { class: 'favicon', alt: '', referrerpolicy: 'no-referrer' });
  try {
    const u = new URL(url);
    if (/^https?:$/.test(u.protocol)) img.src = u.origin + '/favicon.ico';
  } catch (_) {
    // Not a URL; leave the icon blank.
  }
  img.onerror = () => { img.style.visibility = 'hidden'; };
  if (!img.src) img.style.visibility = 'hidden';
  return img;
}

function toast(text) {
  let t = document.querySelector('.toast');
  if (!t) {
    t = el('div', { class: 'toast' });
    document.body.append(t);
  }
  t.textContent = text;
  t.classList.add('show');
  clearTimeout(t._timer);
  t._timer = setTimeout(() => t.classList.remove('show'), 1800);
}

function formatDay(ms) {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86400000);
  if (d.toDateString() === today.toDateString()) return 'Today';
  if (d.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return d.toLocaleDateString(undefined, { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' });
}

function formatTime(ms) {
  return new Date(ms).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

function openUrl(url, e) {
  browser.call('open', { url, newTab: !!(e && (e.ctrlKey || e.metaKey || e.button === 1)) });
}
