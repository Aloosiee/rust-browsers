var window = globalThis;
var self = globalThis;
var top = globalThis;
var parent = globalThis;
var frames = globalThis;

(function () {
  const host = __host;
  const P = host.protos;
  let readyState = 'loading';
  globalThis.__setReadyState = function (s) { readyState = s; };

  function Node() { throw new TypeError('Illegal constructor'); }
  Node.prototype = P.node;
  Object.assign(Node, { ELEMENT_NODE: 1, TEXT_NODE: 3, COMMENT_NODE: 8, DOCUMENT_NODE: 9, DOCUMENT_FRAGMENT_NODE: 11 });
  function Element() { throw new TypeError('Illegal constructor'); }
  Element.prototype = P.element;
  function Text() { throw new TypeError('Illegal constructor'); }
  Text.prototype = P.text;
  function Document() { throw new TypeError('Illegal constructor'); }
  Document.prototype = P.document;
  Object.defineProperty(P.node, 'constructor', { value: Node, writable: true, configurable: true });
  Object.defineProperty(P.element, 'constructor', { value: Element, writable: true, configurable: true });
  Object.defineProperty(P.text, 'constructor', { value: Text, writable: true, configurable: true });
  Object.defineProperty(P.document, 'constructor', { value: Document, writable: true, configurable: true });
  function EventTarget() {}
  Object.assign(globalThis, { Node, Element, Text, Document, EventTarget, HTMLDocument: Document, CharacterData: Text, DocumentFragment: Node });
  for (const n of ['HTMLElement', 'HTMLDivElement', 'HTMLSpanElement', 'HTMLParagraphElement', 'HTMLAnchorElement',
    'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLSelectElement', 'HTMLOptionElement',
    'HTMLFormElement', 'HTMLImageElement', 'HTMLCanvasElement', 'HTMLLIElement', 'HTMLUListElement',
    'HTMLOListElement', 'HTMLTableElement', 'HTMLTableRowElement', 'HTMLTableCellElement', 'HTMLHeadingElement',
    'HTMLScriptElement', 'HTMLStyleElement', 'HTMLLinkElement', 'HTMLBodyElement', 'HTMLHtmlElement',
    'HTMLIFrameElement', 'HTMLVideoElement', 'HTMLMediaElement', 'HTMLTemplateElement', 'HTMLLabelElement',
    'SVGElement', 'Comment']) {
    globalThis[n] = Element;
  }

  function fmt(v, depth = 0) {
    if (typeof v === 'string') return depth ? JSON.stringify(v) : v;
    if (v instanceof Error) return (v.name || 'Error') + ': ' + v.message;
    if (v && typeof v === 'object' && typeof v.nodeType === 'number') {
      if (v.nodeType === 1) return '<' + v.localName + (v.id ? '#' + v.id : '') + '>';
      return v.nodeName;
    }
    if (typeof v === 'function') return 'function ' + (v.name || '(anonymous)');
    if (v && typeof v === 'object') {
      if (depth > 2) return Array.isArray(v) ? '[…]' : '{…}';
      try {
        if (Array.isArray(v)) return '[' + v.map(x => fmt(x, depth + 1)).join(', ') + ']';
        return '{' + Object.keys(v).slice(0, 20).map(k => k + ': ' + fmt(v[k], depth + 1)).join(', ') + '}';
      } catch (e) { return String(v); }
    }
    return String(v);
  }
  const log = level => (...args) => host.log(level, args.map(a => fmt(a)).join(' '));
  const timers = {};
  globalThis.console = {
    log: log('log'), info: log('info'), warn: log('warn'), error: log('error'), debug: log('debug'),
    trace: log('trace'), dir: log('dir'), table: log('table'),
    group: log('group'), groupCollapsed: log('group'), groupEnd() {},
    time(l = 'default') { timers[l] = Date.now(); },
    timeEnd(l = 'default') { host.log('time', l + ': ' + (Date.now() - (timers[l] || Date.now())) + 'ms'); },
    assert(c, ...a) { if (!c) host.log('assert', 'Assertion failed: ' + a.map(x => fmt(x)).join(' ')); },
  };
  globalThis.__reportError = function (e) {
    host.log('error', 'Uncaught ' + fmt(e) + (e && e.stack ? '\n' + e.stack : ''));
  };

  const t0 = Date.now();
  globalThis.performance = { now: () => Date.now() - t0, timeOrigin: t0, mark() {}, measure() {}, getEntriesByType: () => [] };

  function timer(repeat) {
    return function (fn, ms, ...args) {
      const f = typeof fn === 'function' ? fn : new Function(String(fn));
      return host.setTimer(() => { try { f.apply(globalThis, args); } catch (e) { __reportError(e); } }, Number(ms) || 0, repeat);
    };
  }
  globalThis.setTimeout = timer(false);
  globalThis.setInterval = timer(true);
  globalThis.clearTimeout = id => host.clearTimer(id);
  globalThis.clearInterval = id => host.clearTimer(id);
  globalThis.requestAnimationFrame = cb => host.setTimer(() => { try { cb(performance.now()); } catch (e) { __reportError(e); } }, 16, false);
  globalThis.cancelAnimationFrame = id => host.clearTimer(id);
  globalThis.requestIdleCallback = cb => host.setTimer(() => cb({ didTimeout: false, timeRemaining: () => 10 }), 1, false);
  globalThis.cancelIdleCallback = id => host.clearTimer(id);
  globalThis.queueMicrotask = f => Promise.resolve().then(f).catch(__reportError);
  globalThis.structuredClone = v => v === undefined ? undefined : JSON.parse(JSON.stringify(v));

  globalThis.alert = m => host.alert(m === undefined ? '' : String(m));
  globalThis.confirm = m => { host.alert(String(m)); return true; };
  globalThis.prompt = () => null;

  class Event {
    constructor(type, init = {}) {
      this.type = String(type);
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.composed = !!init.composed;
      this.defaultPrevented = false;
      this.target = null;
      this.currentTarget = null;
      this.eventPhase = 0;
      this.isTrusted = false;
      this.timeStamp = performance.now();
    }
    get srcElement() { return this.target; }
    preventDefault() { this.defaultPrevented = true; }
    stopPropagation() { this.__stop = true; }
    stopImmediatePropagation() { this.__stop = true; this.__stopImmediate = true; }
    composedPath() { const p = []; for (let n = this.target; n && n !== window; n = n.parentNode) p.push(n); p.push(window); return p; }
  }
  class CustomEvent extends Event { constructor(t, i = {}) { super(t, i); this.detail = i.detail === undefined ? null : i.detail; } }
  class UIEvent extends Event { constructor(t, i = {}) { super(t, i); this.view = window; this.detail = i.detail || 0; } }
  class MouseEvent extends UIEvent {
    constructor(t, i = {}) {
      super(t, i);
      this.clientX = i.clientX || 0; this.clientY = i.clientY || 0;
      this.pageX = this.clientX; this.pageY = this.clientY + window.scrollY;
      this.screenX = this.clientX; this.screenY = this.clientY;
      this.offsetX = this.clientX; this.offsetY = this.clientY;
      this.button = i.button || 0; this.buttons = i.buttons || 0;
      this.ctrlKey = !!i.ctrlKey; this.shiftKey = !!i.shiftKey; this.altKey = !!i.altKey; this.metaKey = !!i.metaKey;
    }
  }
  class PointerEvent extends MouseEvent { constructor(t, i = {}) { super(t, i); this.pointerId = 1; this.pointerType = 'mouse'; } }
  class KeyboardEvent extends UIEvent {
    constructor(t, i = {}) {
      super(t, i);
      this.key = i.key || ''; this.code = i.code || ''; this.repeat = !!i.repeat;
      this.ctrlKey = !!i.ctrlKey; this.shiftKey = !!i.shiftKey; this.altKey = !!i.altKey; this.metaKey = !!i.metaKey;
      this.keyCode = this.which = this.key.length === 1 ? this.key.toUpperCase().charCodeAt(0) : ({ Enter: 13, Escape: 27, Backspace: 8, Tab: 9, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, ' ': 32 })[this.key] || 0;
    }
  }
  class InputEvent extends UIEvent { constructor(t, i = {}) { super(t, i); this.data = i.data || null; this.inputType = i.inputType || ''; } }
  class FocusEvent extends UIEvent {}
  class SubmitEvent extends Event { constructor(t, i = {}) { super(t, i); this.submitter = i.submitter || null; } }
  Object.assign(globalThis, { Event, CustomEvent, UIEvent, MouseEvent, PointerEvent, KeyboardEvent, InputEvent, FocusEvent, SubmitEvent });

  const listeners = new Map();
  function addL(target, type, fn, opts) {
    if (!fn) return;
    let m = listeners.get(target);
    if (!m) { m = new Map(); listeners.set(target, m); }
    let a = m.get(type);
    if (!a) { a = []; m.set(type, a); }
    const once = !!(opts && typeof opts === 'object' && opts.once);
    if (!a.some(l => l.fn === fn)) a.push({ fn, once });
  }
  function removeL(target, type, fn) {
    const a = listeners.get(target)?.get(type);
    if (!a) return;
    const i = a.findIndex(l => l.fn === fn);
    if (i >= 0) a.splice(i, 1);
  }
  function invoke(node, ev) {
    ev.currentTarget = node;
    const a = listeners.get(node)?.get(ev.type);
    if (a) {
      for (const l of a.slice()) {
        if (l.once) removeL(node, ev.type, l.fn);
        try {
          if (typeof l.fn === 'function') l.fn.call(node, ev); else l.fn.handleEvent(ev);
        } catch (e) { __reportError(e); }
        if (ev.__stopImmediate) return;
      }
    }
    const h = node['on' + ev.type];
    if (typeof h === 'function') {
      try { if (h.call(node, ev) === false) ev.preventDefault(); } catch (e) { __reportError(e); }
    } else if (node !== window && node.nodeType === 1 && node.hasAttribute('on' + ev.type)) {
      try {
        const f = new Function('event', node.getAttribute('on' + ev.type));
        if (f.call(node, ev) === false) ev.preventDefault();
      } catch (e) { __reportError(e); }
    }
  }
  function dispatch(target, ev) {
    ev.target = target;
    const path = [];
    for (let n = target; n && n !== window; n = n.parentNode) path.push(n);
    path.push(window);
    ev.eventPhase = 2;
    invoke(path[0], ev);
    ev.eventPhase = 3;
    if (ev.bubbles) for (let i = 1; i < path.length && !ev.__stop; i++) invoke(path[i], ev);
    ev.currentTarget = null;
    ev.eventPhase = 0;
    return !ev.defaultPrevented;
  }
  globalThis.__hostDispatch = function (target, type, init) {
    const C = init.kind === 'key' ? KeyboardEvent : init.kind === 'mouse' ? MouseEvent : init.kind === 'input' ? InputEvent : Event;
    const ev = new C(type, init);
    ev.isTrusted = true;
    dispatch(target || window, ev);
    return ev.defaultPrevented;
  };
  for (const t of [P.node, EventTarget.prototype]) {
    t.addEventListener = function (type, fn, o) { addL(this, String(type), fn, o); };
    t.removeEventListener = function (type, fn) { removeL(this, String(type), fn); };
    t.dispatchEvent = function (ev) { return dispatch(this, ev); };
  }
  globalThis.addEventListener = (type, fn, o) => addL(window, String(type), fn, o);
  globalThis.removeEventListener = (type, fn) => removeL(window, String(type), fn);
  globalThis.dispatchEvent = ev => dispatch(window, ev);

  function def(proto, name, get, set) {
    Object.defineProperty(proto, name, { get, set, configurable: true, enumerable: true });
  }
  const kebab = p => p.startsWith('--') ? p : p.replace(/[A-Z]/g, c => '-' + c.toLowerCase()).replace(/^(webkit|moz|ms)-/, '-$1-');
  const camel = p => p.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
  function parseStyle(s) {
    const m = new Map();
    for (const d of (s || '').split(';')) {
      const i = d.indexOf(':');
      if (i > 0) m.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return m;
  }
  function setStyle(el, name, v) {
    const m = parseStyle(el.getAttribute('style'));
    if (v === null || v === undefined || v === '') m.delete(name); else m.set(name, String(v));
    const text = [...m].map(([k, x]) => k + ': ' + x + ';').join(' ');
    if (text) el.setAttribute('style', text); else el.removeAttribute('style');
  }
  function styleProxy(el, readOnly) {
    return new Proxy({}, {
      get(_, p) {
        if (typeof p !== 'string') return undefined;
        const m = parseStyle(el.getAttribute('style'));
        switch (p) {
          case 'cssText': return el.getAttribute('style') || '';
          case 'length': return m.size;
          case 'getPropertyValue': return n => m.get(String(n).toLowerCase()) || '';
          case 'getPropertyPriority': return () => '';
          case 'setProperty': return (n, v) => { if (!readOnly) setStyle(el, String(n).toLowerCase(), v); };
          case 'removeProperty': return n => { const o = m.get(n) || ''; if (!readOnly) setStyle(el, n, ''); return o; };
          case 'item': return i => [...m.keys()][i] || '';
          case 'toJSON': return () => Object.fromEntries([...m].map(([k, v]) => [camel(k), v]));
        }
        return m.get(kebab(p)) || '';
      },
      set(_, p, v) {
        if (readOnly) return true;
        if (p === 'cssText') { el.setAttribute('style', String(v)); return true; }
        setStyle(el, kebab(String(p)), v);
        return true;
      },
    });
  }
  def(P.element, 'style', function () { return styleProxy(this, false); }, function (v) { this.setAttribute('style', String(v)); });
  globalThis.getComputedStyle = el => styleProxy(el, true);

  def(P.element, 'classList', function () {
    const el = this;
    const get = () => (el.getAttribute('class') || '').split(/\s+/).filter(Boolean);
    const save = a => el.setAttribute('class', a.join(' '));
    return {
      add(...c) { const a = get(); for (const x of c) if (!a.includes(x)) a.push(x); save(a); },
      remove(...c) { save(get().filter(x => !c.includes(x))); },
      toggle(c, force) {
        const a = get(); const has = a.includes(c); const want = force === undefined ? !has : !!force;
        if (want && !has) a.push(c);
        if (!want && has) a.splice(a.indexOf(c), 1);
        save(a); return want;
      },
      contains(c) { return get().includes(c); },
      replace(o, n) { const a = get(); const i = a.indexOf(o); if (i < 0) return false; a[i] = n; save(a); return true; },
      item(i) { return get()[i] ?? null; },
      get length() { return get().length; },
      get value() { return el.getAttribute('class') || ''; },
      toString() { return el.getAttribute('class') || ''; },
      forEach(f) { get().forEach(f); },
      [Symbol.iterator]() { return get()[Symbol.iterator](); },
    };
  });
  def(P.element, 'dataset', function () {
    const el = this;
    return new Proxy({}, {
      get(_, p) { if (typeof p !== 'string') return undefined; const v = el.getAttribute('data-' + kebab(p)); return v === null ? undefined : v; },
      set(_, p, v) { el.setAttribute('data-' + kebab(String(p)), String(v)); return true; },
      deleteProperty(_, p) { el.removeAttribute('data-' + kebab(String(p))); return true; },
      has(_, p) { return el.hasAttribute('data-' + kebab(String(p))); },
      ownKeys() { return el.getAttributeNames().filter(n => n.startsWith('data-')).map(n => camel(n.slice(5))); },
      getOwnPropertyDescriptor(_, p) { const v = el.getAttribute('data-' + kebab(String(p))); return v === null ? undefined : { value: v, enumerable: true, configurable: true, writable: true }; },
    });
  });
  def(P.element, 'attributes', function () {
    return this.getAttributeNames().map(name => ({ name, value: this.getAttribute(name) }));
  });
  def(P.element, 'innerText', function () { return this.textContent; }, function (v) { this.textContent = v; });
  def(P.element, 'outerText', function () { return this.textContent; });
  for (const p of ['hidden', 'disabled', 'required', 'readOnly', 'multiple', 'selected', 'autofocus', 'open', 'defer', 'async', 'noValidate']) {
    const a = p.toLowerCase();
    def(P.element, p, function () { return this.hasAttribute(a); }, function (v) { if (v) this.setAttribute(a, ''); else this.removeAttribute(a); });
  }
  const reflect = { title: 'title', name: 'name', type: 'type', placeholder: 'placeholder', alt: 'alt', htmlFor: 'for', rel: 'rel', target: 'target', method: 'method', lang: 'lang', dir: 'dir', role: 'role', min: 'min', max: 'max', step: 'step', pattern: 'pattern', autocomplete: 'autocomplete', defaultValue: 'value' };
  for (const [p, a] of Object.entries(reflect)) {
    def(P.element, p, function () { return this.getAttribute(a) ?? ''; }, function (v) { this.setAttribute(a, String(v)); });
  }
  for (const p of ['href', 'src', 'action']) {
    def(P.element, p, function () { const v = this.getAttribute(p); return v === null ? '' : host.resolve(v); }, function (v) { this.setAttribute(p, String(v)); });
  }
  def(P.element, 'tabIndex', function () { return parseInt(this.getAttribute('tabindex') ?? '-1', 10); }, function (v) { this.setAttribute('tabindex', String(v)); });
  def(P.element, 'options', function () { return this.querySelectorAll('option'); });
  def(P.element, 'selectedIndex', function () {
    const o = this.querySelectorAll('option'); const i = o.findIndex(x => x.hasAttribute('selected')); return i < 0 ? (o.length ? 0 : -1) : i;
  }, function (i) { this.querySelectorAll('option').forEach((o, k) => o.selected = k === i); });
  def(P.element, 'form', function () { return this.closest('form'); });
  def(P.element, 'elements', function () { return this.querySelectorAll('input, select, textarea, button'); });
  def(P.element, 'offsetWidth', function () { return Math.round(this.getBoundingClientRect().width); });
  def(P.element, 'offsetHeight', function () { return Math.round(this.getBoundingClientRect().height); });
  def(P.element, 'offsetTop', function () { return Math.round(this.getBoundingClientRect().top + window.scrollY); });
  def(P.element, 'offsetLeft', function () { return Math.round(this.getBoundingClientRect().left); });
  def(P.element, 'clientWidth', function () { return this.offsetWidth; });
  def(P.element, 'clientHeight', function () { return this.offsetHeight; });
  def(P.element, 'scrollWidth', function () { return this.offsetWidth; });
  def(P.element, 'scrollHeight', function () { return this.offsetHeight; });
  def(P.element, 'scrollTop', function () { return 0; }, function () {});
  def(P.element, 'scrollLeft', function () { return 0; }, function () {});
  def(P.element, 'offsetParent', function () { return document.body; });
  def(P.node, 'ownerDocument', function () { return this === document ? null : document; });
  def(P.node, 'baseURI', function () { return location.href; });

  const toNode = x => (x && typeof x === 'object' && typeof x.nodeType === 'number') ? x : document.createTextNode(String(x));
  Object.assign(P.node, {
    append(...xs) { for (const x of xs) this.appendChild(toNode(x)); },
    prepend(...xs) { const first = this.firstChild; for (const x of xs) this.insertBefore(toNode(x), first); },
    replaceChildren(...xs) { while (this.firstChild) this.removeChild(this.firstChild); this.append(...xs); },
    before(...xs) { const p = this.parentNode; if (p) for (const x of xs) p.insertBefore(toNode(x), this); },
    after(...xs) { const p = this.parentNode; if (!p) return; const next = this.nextSibling; for (const x of xs) p.insertBefore(toNode(x), next); },
    replaceWith(...xs) { const p = this.parentNode; if (!p) return; this.before(...xs); p.removeChild(this); },
    isEqualNode(o) { return !!o && this.outerHTML === o.outerHTML; },
    isSameNode(o) { return this === o; },
    getRootNode() { let n = this; while (n.parentNode) n = n.parentNode; return n; },
    normalize() {},
    compareDocumentPosition(o) { return this.contains(o) ? 20 : o.contains(this) ? 10 : 4; },
  });
  Object.assign(P.element, {
    toggleAttribute(n, force) { const has = this.hasAttribute(n); const want = force === undefined ? !has : !!force; if (want) this.setAttribute(n, ''); else this.removeAttribute(n); return want; },
    insertAdjacentElement(pos, el) {
      switch (String(pos).toLowerCase()) {
        case 'beforebegin': this.before(el); break;
        case 'afterbegin': this.prepend(el); break;
        case 'beforeend': this.append(el); break;
        case 'afterend': this.after(el); break;
      }
      return el;
    },
    insertAdjacentText(pos, t) { this.insertAdjacentElement(pos, document.createTextNode(t)); },
    click() { this.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })); },
    focus() { host.focus(this); this.dispatchEvent(new FocusEvent('focus')); },
    blur() { host.focus(null); this.dispatchEvent(new FocusEvent('blur')); },
    select() {}, setSelectionRange() {}, scrollIntoView() { window.scrollTo(0, this.offsetTop); },
    submit() { host.log('info', 'form.submit() is not supported'); },
    reset() {}, checkValidity() { return true; }, reportValidity() { return true; },
    getClientRects() { return [this.getBoundingClientRect()]; },
    attachShadow() { return this; },
    animate() { return { finished: Promise.resolve(), cancel() {}, play() {}, pause() {}, onfinish: null }; },
    getAnimations() { return []; },
  });

  Object.defineProperty(document, 'documentElement', { get() { return this.querySelector('html'); }, configurable: true });
  Object.defineProperty(document, 'body', { get() { return this.querySelector('body'); }, configurable: true });
  Object.defineProperty(document, 'head', { get() { return this.querySelector('head'); }, configurable: true });
  Object.defineProperty(document, 'title', {
    get() { const t = this.querySelector('title'); return t ? t.textContent.trim().replace(/\s+/g, ' ') : ''; },
    set(v) {
      let t = this.querySelector('title');
      if (!t) { t = this.createElement('title'); (this.head || this.documentElement).appendChild(t); }
      t.textContent = String(v);
    }, configurable: true,
  });
  let cookies = '';
  Object.defineProperties(document, {
    readyState: { get: () => readyState, configurable: true },
    cookie: { get: () => cookies, set: v => { const kv = String(v).split(';')[0]; cookies = cookies ? cookies + '; ' + kv : kv; }, configurable: true },
    location: { get: () => location, set: v => host.navigate(String(v)), configurable: true },
    URL: { get: () => location.href, configurable: true },
    domain: { get: () => location.hostname, configurable: true },
    referrer: { get: () => '', configurable: true },
    defaultView: { get: () => window, configurable: true },
    activeElement: { get: () => host.activeElement(), configurable: true },
    forms: { get() { return this.querySelectorAll('form'); }, configurable: true },
    links: { get() { return this.querySelectorAll('a[href]'); }, configurable: true },
    images: { get() { return this.querySelectorAll('img'); }, configurable: true },
    scripts: { get() { return this.querySelectorAll('script'); }, configurable: true },
    visibilityState: { get: () => 'visible', configurable: true },
    hidden: { get: () => false, configurable: true },
    characterSet: { get: () => 'UTF-8', configurable: true },
    compatMode: { get: () => 'CSS1Compat', configurable: true },
    fonts: { get: () => ({ ready: Promise.resolve(), add() {}, load: () => Promise.resolve([]) }), configurable: true },
    scrollingElement: { get() { return this.documentElement; }, configurable: true },
  });
  document.writeln = function (...a) { document.write(...a, '\n'); };
  document.open = function () {};
  document.close = function () {};
  document.getElementsByName = function (n) { return this.querySelectorAll('[name="' + String(n).replace(/"/g, '\\"') + '"]'); };
  document.hasFocus = () => true;
  document.createEvent = () => new Event('');
  document.createElementNS = (ns, tag) => document.createElement(tag);
  document.elementFromPoint = () => null;
  document.execCommand = () => false;

  const loc = {};
  for (const k of ['href', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash', 'origin']) {
    Object.defineProperty(loc, k, {
      get: () => host.location()[k],
      set: v => {
        if (k === 'href') return host.navigate(String(v));
        if (k === 'hash') { const h = String(v); return host.navigate(location.href.split('#')[0] + (h.startsWith('#') ? h : '#' + h)); }
        host.log('warn', 'Setting location.' + k + ' is not supported');
      },
      enumerable: true,
    });
  }
  Object.assign(loc, {
    assign: u => host.navigate(String(u)),
    replace: u => host.navigate(String(u)),
    reload: () => host.navigate(location.href),
    toString: () => location.href,
  });
  Object.defineProperty(globalThis, 'location', { get: () => loc, set: v => host.navigate(String(v)), configurable: true });

  globalThis.history = {
    get length() { return 1; },
    state: null,
    back: () => host.history(-1),
    forward: () => host.history(1),
    go: n => host.history(n || 0),
    pushState(s, t, u) { this.state = s; if (u != null) host.pushState(String(u)); },
    replaceState(s, t, u) { this.state = s; if (u != null) host.pushState(String(u)); },
    scrollRestoration: 'auto',
  };
  globalThis.navigator = {
    userAgent: 'Mozilla/5.0 (compatible; RustBrowser/0.2)', appName: 'Netscape', appVersion: '5.0',
    platform: 'Linux x86_64', vendor: '', language: 'en-US', languages: ['en-US', 'en'],
    onLine: true, cookieEnabled: true, hardwareConcurrency: 2, maxTouchPoints: 0, webdriver: false,
    doNotTrack: null, javaEnabled: () => false, sendBeacon: () => true,
    clipboard: { writeText: () => Promise.resolve(), readText: () => Promise.resolve('') },
    serviceWorker: undefined, mediaDevices: undefined, geolocation: undefined,
  };
  globalThis.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1080, colorDepth: 24, pixelDepth: 24, orientation: { type: 'landscape-primary', angle: 0 } };
  Object.defineProperties(globalThis, {
    innerWidth: { get: () => host.viewport()[0], configurable: true },
    innerHeight: { get: () => host.viewport()[1], configurable: true },
    outerWidth: { get: () => host.viewport()[0], configurable: true },
    outerHeight: { get: () => host.viewport()[1] + 80, configurable: true },
    scrollY: { get: () => host.viewport()[2], configurable: true },
    pageYOffset: { get: () => host.viewport()[2], configurable: true },
    scrollX: { get: () => 0, configurable: true },
    pageXOffset: { get: () => 0, configurable: true },
    devicePixelRatio: { get: () => 1, configurable: true },
    isSecureContext: { get: () => location.protocol === 'https:', configurable: true },
    origin: { get: () => location.origin, configurable: true },
  });
  globalThis.scrollTo = (x, y) => host.scrollTo(typeof x === 'object' && x ? (x.top ?? window.scrollY) : (y ?? 0));
  globalThis.scroll = globalThis.scrollTo;
  globalThis.scrollBy = (x, y) => host.scrollTo(window.scrollY + (typeof x === 'object' && x ? (x.top || 0) : (y || 0)));
  globalThis.matchMedia = q => {
    const m = { media: String(q), matches: host.mediaMatches(String(q)), onchange: null };
    m.addListener = m.removeListener = m.addEventListener = m.removeEventListener = () => {};
    return m;
  };
  globalThis.focus = globalThis.blur = globalThis.print = globalThis.stop = () => {};
  globalThis.open = (u) => { if (u) host.navigate(String(u)); return null; };
  globalThis.close = () => {};
  globalThis.getSelection = () => ({ toString: () => '', removeAllRanges() {}, addRange() {}, rangeCount: 0 });

  class Storage {
    constructor(session) { Object.defineProperty(this, '_s', { value: session }); }
    getItem(k) { return host.storage('get', String(k), '', this._s); }
    setItem(k, v) { host.storage('set', String(k), String(v), this._s); }
    removeItem(k) { host.storage('remove', String(k), '', this._s); }
    clear() { host.storage('clear', '', '', this._s); }
    key(i) { const k = host.storage('keys', '', '', this._s); return (k ? k.split('\u0001') : [])[i] ?? null; }
    get length() { const k = host.storage('keys', '', '', this._s); return k ? k.split('\u0001').length : 0; }
  }
  globalThis.Storage = Storage;
  globalThis.localStorage = new Storage(false);
  globalThis.sessionStorage = new Storage(true);

  class URLSearchParams {
    constructor(init = '') {
      this._p = [];
      if (typeof init === 'string') {
        for (const part of init.replace(/^\?/, '').split('&')) {
          if (!part) continue;
          const i = part.indexOf('=');
          const dec = s => decodeURIComponent(s.replace(/\+/g, ' '));
          this._p.push(i < 0 ? [dec(part), ''] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
        }
      } else if (init && typeof init === 'object') {
        for (const [k, v] of (Array.isArray(init) ? init : Object.entries(init))) this._p.push([String(k), String(v)]);
      }
    }
    get(k) { const e = this._p.find(p => p[0] === k); return e ? e[1] : null; }
    getAll(k) { return this._p.filter(p => p[0] === k).map(p => p[1]); }
    has(k) { return this._p.some(p => p[0] === k); }
    set(k, v) { this.delete(k); this._p.push([String(k), String(v)]); }
    append(k, v) { this._p.push([String(k), String(v)]); }
    delete(k) { this._p = this._p.filter(p => p[0] !== k); }
    forEach(f) { this._p.forEach(([k, v]) => f(v, k, this)); }
    entries() { return this._p[Symbol.iterator](); }
    keys() { return this._p.map(p => p[0])[Symbol.iterator](); }
    values() { return this._p.map(p => p[1])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    toString() { return this._p.map(([k, v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&'); }
  }
  class URL {
    constructor(u, base) {
      Object.assign(this, host.parseUrl(String(u), base === undefined ? undefined : String(base)));
      this.searchParams = new URLSearchParams(this.search);
      this.username = ''; this.password = '';
    }
    toString() { return this.href; }
    toJSON() { return this.href; }
    static canParse(u, b) { try { new URL(u, b); return true; } catch (e) { return false; } }
    static createObjectURL() { return 'blob:unsupported'; }
    static revokeObjectURL() {}
  }
  Object.assign(globalThis, { URL, URLSearchParams });

  class Headers {
    constructor(init = {}) { this._h = {}; for (const [k, v] of Object.entries(init || {})) this._h[k.toLowerCase()] = String(v); }
    get(k) { return this._h[String(k).toLowerCase()] ?? null; }
    has(k) { return String(k).toLowerCase() in this._h; }
    set(k, v) { this._h[String(k).toLowerCase()] = String(v); }
    append(k, v) { this.set(k, v); }
    forEach(f) { for (const [k, v] of Object.entries(this._h)) f(v, k, this); }
  }
  class Response {
    constructor(r) {
      this.status = r.status; this.ok = r.status >= 200 && r.status < 300; this.statusText = this.ok ? 'OK' : '';
      this.url = r.url; this.redirected = false; this.type = 'basic';
      this.headers = new Headers({ 'content-type': r.contentType });
      this._body = r.body; this.bodyUsed = false;
    }
    text() { this.bodyUsed = true; return Promise.resolve(this._body); }
    json() { this.bodyUsed = true; try { return Promise.resolve(JSON.parse(this._body)); } catch (e) { return Promise.reject(e); } }
    clone() { return new Response({ status: this.status, url: this.url, contentType: this.headers.get('content-type'), body: this._body }); }
  }
  globalThis.Headers = Headers;
  globalThis.Response = Response;
  globalThis.fetch = function (input, opts = {}) {
    return new Promise((resolve, reject) => {
      try {
        const url = typeof input === 'string' ? input : (input && (input.url || input.href)) || String(input);
        resolve(new Response(host.fetchSync(url, opts.method || 'GET', opts.body == null ? null : String(opts.body))));
      } catch (e) {
        reject(new TypeError('Failed to fetch: ' + e.message));
      }
    });
  };

  class XMLHttpRequest {
    constructor() { this.readyState = 0; this.status = 0; this.responseText = ''; this.response = ''; this.responseType = ''; this._l = {}; this.onload = this.onerror = this.onreadystatechange = null; }
    open(method, url, async = true) { this._m = method; this._u = url; this._async = async !== false; this.readyState = 1; }
    setRequestHeader() {}
    getResponseHeader(n) { return String(n).toLowerCase() === 'content-type' ? this._ct || null : null; }
    getAllResponseHeaders() { return this._ct ? 'content-type: ' + this._ct + '\r\n' : ''; }
    addEventListener(t, f) { (this._l[t] = this._l[t] || []).push(f); }
    removeEventListener(t, f) { this._l[t] = (this._l[t] || []).filter(x => x !== f); }
    _fire(t) { const ev = new Event(t); ev.target = this; const h = this['on' + t]; if (h) try { h.call(this, ev); } catch (e) { __reportError(e); } for (const f of this._l[t] || []) try { f.call(this, ev); } catch (e) { __reportError(e); } }
    send(body) {
      const run = () => {
        try {
          const r = host.fetchSync(this._u, this._m, body == null ? null : String(body));
          this.status = r.status; this.responseText = r.body; this._ct = r.contentType; this.responseURL = r.url;
          this.response = this.responseType === 'json' ? (() => { try { return JSON.parse(r.body); } catch (e) { return null; } })() : r.body;
          this.readyState = 4; this._fire('readystatechange'); this._fire('load'); this._fire('loadend');
        } catch (e) {
          this.readyState = 4; this.status = 0; this._fire('readystatechange'); this._fire('error'); this._fire('loadend');
        }
      };
      if (this._async) setTimeout(run, 0); else run();
    }
    abort() {}
  }
  Object.assign(XMLHttpRequest, { UNSENT: 0, OPENED: 1, HEADERS_RECEIVED: 2, LOADING: 3, DONE: 4 });
  globalThis.XMLHttpRequest = XMLHttpRequest;

  class MutationObserver { constructor(cb) {} observe() {} disconnect() {} takeRecords() { return []; } }
  class ResizeObserver { constructor(cb) {} observe() {} unobserve() {} disconnect() {} }
  class IntersectionObserver {
    constructor(cb) { this._cb = cb; }
    observe(el) { setTimeout(() => { try { this._cb([{ target: el, isIntersecting: true, intersectionRatio: 1, boundingClientRect: el.getBoundingClientRect() }], this); } catch (e) { __reportError(e); } }, 0); }
    unobserve() {} disconnect() {} takeRecords() { return []; }
  }
  Object.assign(globalThis, { MutationObserver, ResizeObserver, IntersectionObserver });
  globalThis.customElements = { define() {}, get() { return undefined; }, whenDefined: () => Promise.resolve() };
  globalThis.Image = function (w, h) { const i = document.createElement('img'); if (w) i.setAttribute('width', w); if (h) i.setAttribute('height', h); return i; };
  globalThis.Option = function (text = '', value) { const o = document.createElement('option'); o.textContent = text; if (value !== undefined) o.setAttribute('value', value); return o; };
  globalThis.DOMParser = class { parseFromString() { return document; } };
  globalThis.AbortController = class { constructor() { this.signal = { aborted: false, addEventListener() {}, removeEventListener() {} }; } abort() { this.signal.aborted = true; } };

  const b64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = s => {
    s = String(s); let out = '';
    for (let i = 0; i < s.length; i += 3) {
      const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error('InvalidCharacterError');
      const n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += b64[(n >> 18) & 63] + b64[(n >> 12) & 63] + (i + 1 < s.length ? b64[(n >> 6) & 63] : '=') + (i + 2 < s.length ? b64[n & 63] : '=');
    }
    return out;
  };
  globalThis.atob = s => {
    s = String(s).replace(/[\s=]/g, ''); let out = '';
    for (let i = 0; i < s.length; i += 4) {
      const n = (b64.indexOf(s[i]) << 18) | (b64.indexOf(s[i + 1]) << 12) | ((b64.indexOf(s[i + 2]) & 63) << 6) | (b64.indexOf(s[i + 3]) & 63);
      out += String.fromCharCode((n >> 16) & 255);
      if (i + 2 < s.length) out += String.fromCharCode((n >> 8) & 255);
      if (i + 3 < s.length) out += String.fromCharCode(n & 255);
    }
    return out;
  };
})();
