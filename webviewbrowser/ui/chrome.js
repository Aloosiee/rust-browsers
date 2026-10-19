// The browser toolbar: tab strip, navigation bar, bookmarks bar, find bar and the address
// bar's suggestion list. Rust pushes state in through `window.ui`; user actions go out as
// JSON messages over the IPC bridge.
'use strict';

const $ = (id) => document.getElementById(id);
const send = (message) => window.ipc.postMessage(JSON.stringify(message));
const action = (name) => send({ kind: 'action', name });

const GLOBE = '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.2"/><path d="M1.8 8h12.4M8 1.8c2 2 2 10.4 0 12.4M8 1.8c-2 2-2 10.4 0 12.4" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>';
const SEARCH = '<svg viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="m10.5 10.5 3 3" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>';
const STAR = '<svg viewBox="0 0 16 16"><path d="m8 1.8 1.9 3.9 4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6z" fill="currentColor"/></svg>';
const CLOCK = '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M8 4.5V8l2.5 1.5" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>';
const CLOSE = '<svg viewBox="0 0 16 16"><path d="m4 4 8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>';
const APP_ICON = 'icon.svg';

let state = {
  tabs: [], url: '', canBack: false, canForward: false, loading: false, bookmarked: false,
  zoom: 1, bookmarks: [], showBookmarksBar: true, downloads: { active: 0, recent: null },
  private: false, searchEngine: 'DuckDuckGo', internal: false,
};
let editing = false;
let suggestionItems = [];
let selected = -1;
let lastTyped = '';
let dragId = null;

function iconElement(src, fallback) {
  const span = document.createElement('span');
  span.className = 'favicon';
  if (src) {
    const img = document.createElement('img');
    img.src = src;
    img.alt = '';
    img.referrerPolicy = 'no-referrer';
    img.onerror = () => { span.innerHTML = fallback; };
    span.appendChild(img);
  } else {
    span.innerHTML = fallback;
  }
  return span;
}

function renderTabs() {
  if (dragId !== null) return;
  const container = $('tabs');
  container.textContent = '';
  const narrow = state.tabs.length > 10;
  for (const tab of state.tabs) {
    const el = document.createElement('div');
    el.className = 'tab' + (tab.active ? ' active' : '') + (narrow ? ' narrow' : '');
    el.setAttribute('role', 'tab');
    el.setAttribute('aria-selected', tab.active ? 'true' : 'false');
    el.title = tab.title + (tab.url ? '\n' + tab.url : '');
    el.draggable = true;
    el.dataset.id = tab.id;

    if (tab.loading) {
      const spinner = document.createElement('span');
      spinner.className = 'favicon';
      spinner.innerHTML = '<span class="spinner"></span>';
      el.appendChild(spinner);
    } else {
      el.appendChild(iconElement(tab.internal ? APP_ICON : tab.favicon, GLOBE));
    }

    const title = document.createElement('span');
    title.className = 'title';
    title.textContent = tab.title || tab.url || 'New Tab';
    el.appendChild(title);

    const close = document.createElement('button');
    close.className = 'icon close';
    close.title = 'Close tab';
    close.innerHTML = CLOSE;
    close.addEventListener('click', (e) => { e.stopPropagation(); send({ kind: 'closeTab', id: tab.id }); });
    el.appendChild(close);

    el.addEventListener('mousedown', (e) => {
      if (e.button === 0) send({ kind: 'selectTab', id: tab.id });
    });
    el.addEventListener('auxclick', (e) => {
      if (e.button === 1) send({ kind: 'closeTab', id: tab.id });
    });
    el.addEventListener('dblclick', (e) => e.stopPropagation());
    el.addEventListener('dragstart', (e) => {
      dragId = tab.id;
      el.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', tab.url || '');
    });
    el.addEventListener('dragend', () => { dragId = null; renderTabs(); });
    el.addEventListener('dragover', (e) => { if (dragId !== null) e.preventDefault(); });
    el.addEventListener('drop', (e) => {
      e.preventDefault();
      if (dragId === null || dragId === tab.id) return;
      const index = state.tabs.findIndex((t) => t.id === tab.id);
      send({ kind: 'moveTab', id: dragId, index });
    });
    container.appendChild(el);
    if (tab.active) requestAnimationFrame(() => el.scrollIntoView({ inline: 'nearest' }));
  }
}

function renderBookmarks() {
  const bar = $('bookmarksbar');
  bar.hidden = !state.showBookmarksBar;
  bar.textContent = '';
  if (state.bookmarks.length === 0) {
    const empty = document.createElement('span');
    empty.className = 'empty';
    empty.textContent = 'Bookmark pages with the star in the address bar to see them here.';
    bar.appendChild(empty);
    return;
  }
  for (const b of state.bookmarks) {
    const el = document.createElement('div');
    el.className = 'bookmark';
    el.title = b.title + '\n' + b.url;
    el.appendChild(iconElement(faviconFor(b.url), GLOBE));
    const label = document.createElement('span');
    label.textContent = b.title || b.url;
    el.appendChild(label);
    el.addEventListener('click', (e) => send({ kind: 'openUrl', url: b.url, newTab: e.ctrlKey || e.metaKey }));
    el.addEventListener('auxclick', (e) => { if (e.button === 1) send({ kind: 'openUrl', url: b.url, newTab: true }); });
    bar.appendChild(el);
  }
}

function faviconFor(url) {
  try {
    const u = new URL(url);
    return /^https?:$/.test(u.protocol) ? u.origin + '/favicon.ico' : '';
  } catch (_) {
    return '';
  }
}

function render() {
  document.body.classList.toggle('private', state.private);
  $('private-badge').hidden = !state.private;
  renderTabs();
  $('back').disabled = !state.canBack;
  $('forward').disabled = !state.canForward;
  $('reload').classList.toggle('loading', state.loading);
  $('reload').title = state.loading ? 'Stop loading this page (Esc)' : 'Reload current page (Ctrl+R)';
  if (!editing) $('url').value = state.url;
  const active = state.tabs.find((t) => t.active);
  const siteIcon = $('site-icon');
  siteIcon.textContent = '';
  siteIcon.appendChild(iconElement(state.internal ? APP_ICON : (active && active.favicon) || '', state.url ? GLOBE : SEARCH));
  $('star').classList.toggle('on', state.bookmarked);
  $('star').title = state.bookmarked ? 'Remove bookmark (Ctrl+D)' : 'Bookmark this page (Ctrl+D)';
  $('star').hidden = !state.url;
  const zoom = $('zoom');
  zoom.hidden = Math.abs(state.zoom - (state.defaultZoom || 1)) < 0.001;
  zoom.textContent = Math.round(state.zoom * 100) + '%';
  const badge = $('downloads-badge');
  badge.hidden = state.downloads.active === 0;
  badge.textContent = state.downloads.active;
  const dl = $('downloads');
  dl.classList.toggle('done', state.downloads.active === 0 && !!state.downloads.recent);
  dl.title = state.downloads.recent
    ? `Downloads (Ctrl+J)\n${state.downloads.recent.name}: ${state.downloads.recent.state}`
    : 'Downloads (Ctrl+J)';
  renderBookmarks();
}

/* Address bar */

const url = $('url');

url.addEventListener('focus', () => {
  editing = true;
  requestAnimationFrame(() => url.select());
});
url.addEventListener('blur', () => {
  editing = false;
  closeSuggestions();
  url.value = state.url;
  url.scrollLeft = 0;
});
url.addEventListener('input', () => {
  lastTyped = url.value;
  selected = -1;
  send({ kind: 'suggest', text: url.value });
});
url.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    if (!suggestionItems.length) return;
    e.preventDefault();
    const n = suggestionItems.length;
    selected = e.key === 'ArrowDown' ? (selected + 1) % n : (selected - 1 + n) % n;
    url.value = suggestionItems[selected].fill || lastTyped;
    renderSuggestions();
  } else if (e.key === 'Enter') {
    e.preventDefault();
    const item = selected >= 0 ? suggestionItems[selected] : null;
    go(item, e.altKey);
  } else if (e.key === 'Escape') {
    e.preventDefault();
    if (!$('suggestions').hidden) {
      closeSuggestions();
      url.value = lastTyped || state.url;
    } else {
      url.value = state.url;
      send({ kind: 'focusPage' });
    }
  }
});

function go(item, newTab) {
  const input = item ? item.input : url.value;
  if (!input.trim()) return;
  editing = false;
  closeSuggestions();
  send({ kind: 'navigate', input, newTab: !!newTab });
}

function closeSuggestions() {
  suggestionItems = [];
  selected = -1;
  renderSuggestions();
}

function renderSuggestions() {
  const box = $('suggestions');
  box.textContent = '';
  if (!suggestionItems.length || document.activeElement !== url) {
    box.hidden = true;
    reportOverlay();
    return;
  }
  box.hidden = false;
  suggestionItems.forEach((item, i) => {
    const row = document.createElement('div');
    row.className = 'suggestion' + (i === selected ? ' selected' : '');
    row.setAttribute('role', 'option');
    const glyph = document.createElement('span');
    glyph.className = 'glyph';
    glyph.innerHTML = item.type === 'search' ? SEARCH : item.type === 'bookmark' ? STAR : item.type === 'history' ? CLOCK : GLOBE;
    row.appendChild(glyph);
    const text = document.createElement('span');
    text.className = 'text';
    text.textContent = item.title;
    row.appendChild(text);
    if (item.url && item.type !== 'search') {
      const sep = document.createElement('span');
      sep.className = 'sep';
      sep.textContent = '—';
      row.appendChild(sep);
      const link = document.createElement('span');
      link.className = 'url';
      link.textContent = item.url;
      row.appendChild(link);
    }
    row.addEventListener('mousedown', (e) => { e.preventDefault(); go(item, e.button === 1); });
    box.appendChild(row);
  });
  reportOverlay();
}

let reportedOverlay = -1;
function reportOverlay() {
  const box = $('suggestions');
  const height = box.hidden ? 0 : Math.ceil(box.getBoundingClientRect().height);
  if (height !== reportedOverlay) {
    reportedOverlay = height;
    send({ kind: 'overlay', height });
  }
}

let reportedLayout = -1;
new ResizeObserver(() => {
  const height = Math.ceil($('bars').getBoundingClientRect().height);
  if (height !== reportedLayout) {
    reportedLayout = height;
    send({ kind: 'layout', height });
  }
}).observe($('bars'));

/* Buttons */

$('new-tab').addEventListener('click', () => action('newTab'));
$('tabstrip').addEventListener('dblclick', (e) => { if (e.target.id === 'tabstrip' || e.target.id === 'tabs') action('newTab'); });
$('back').addEventListener('click', () => action('back'));
$('forward').addEventListener('click', () => action('forward'));
$('reload').addEventListener('click', () => action(state.loading ? 'stop' : 'reload'));
$('home').addEventListener('click', () => action('home'));
$('star').addEventListener('click', () => action('bookmarkPage'));
$('zoom').addEventListener('click', () => action('zoomReset'));
$('downloads').addEventListener('click', () => action('showDownloads'));
$('menu').addEventListener('click', () => {
  const r = $('menu').getBoundingClientRect();
  send({ kind: 'menu', x: r.right, y: r.bottom });
});

/* Find bar */

const find = $('find');
let findTimer = 0;
find.addEventListener('input', () => {
  clearTimeout(findTimer);
  findTimer = setTimeout(() => send({ kind: 'find', query: find.value, backwards: false, again: false }), 120);
});
find.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    e.preventDefault();
    send({ kind: 'find', query: find.value, backwards: e.shiftKey, again: true });
  } else if (e.key === 'Escape') {
    e.preventDefault();
    send({ kind: 'findClose' });
  }
});
$('find-next').addEventListener('click', () => send({ kind: 'find', query: find.value, backwards: false, again: true }));
$('find-prev').addEventListener('click', () => send({ kind: 'find', query: find.value, backwards: true, again: true }));
$('find-close').addEventListener('click', () => send({ kind: 'findClose' }));

/* Info bar */

let infobarId = null;
$('infobar-accept').addEventListener('click', () => send({ kind: 'infobar', id: infobarId, accept: true }));
$('infobar-cancel').addEventListener('click', () => send({ kind: 'infobar', id: infobarId, accept: false }));

/* Shortcuts while the toolbar has focus */

window.addEventListener('keydown', (e) => {
  const hit = BrowserShortcuts.match(e);
  if (!hit) return;
  const inField = e.target === url || e.target === find;
  if (inField && !hit.reserved && ['back', 'forward', 'stop'].includes(hit.action)) return;
  if (hit.action === 'find' && document.activeElement === find) {
    e.preventDefault();
    find.select();
    return;
  }
  e.preventDefault();
  action(hit.action);
}, true);

/* API used by Rust */

window.ui = {
  state(next) {
    state = next;
    render();
  },
  suggestions(text, items) {
    if (text !== url.value && text !== lastTyped) return;
    suggestionItems = items;
    selected = -1;
    renderSuggestions();
  },
  focusAddressBar() {
    url.focus();
    url.select();
  },
  openFind(prefill) {
    $('findbar').hidden = false;
    if (prefill) find.value = prefill;
    find.focus();
    find.select();
    if (find.value) send({ kind: 'find', query: find.value, backwards: false, again: false });
  },
  closeFind() {
    $('findbar').hidden = true;
    $('find-status').textContent = '';
    find.classList.remove('notfound');
  },
  findValue() {
    return find.value;
  },
  findResult(current, total) {
    const status = $('find-status');
    find.classList.toggle('notfound', !!find.value && total === 0);
    if (!find.value) status.textContent = '';
    else if (total === 0) status.textContent = 'Phrase not found';
    else status.textContent = `${current} of ${total} match${total === 1 ? '' : 'es'}`;
  },
  infobar(info) {
    const bar = $('infobar');
    if (!info) {
      bar.hidden = true;
      infobarId = null;
      return;
    }
    infobarId = info.id;
    $('infobar-text').textContent = info.message;
    $('infobar-accept').textContent = info.accept;
    $('infobar-cancel').textContent = info.cancel;
    bar.hidden = false;
  },
};

document.addEventListener('contextmenu', (e) => {
  if (e.target !== url && e.target !== find) e.preventDefault();
});

send({ kind: 'ready' });
