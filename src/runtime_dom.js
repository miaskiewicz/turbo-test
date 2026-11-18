// DOM-only fallback APIs; evaluate after the shared runtime and browser environment.
// ---- minimal Web Storage. A real `Storage` class (not an object literal) so the methods live on
// `Storage.prototype` — tests do `vi.spyOn(Storage.prototype, 'setItem')`, which only intercepts
// localStorage/sessionStorage calls when those are genuine Storage instances. ----
class __Storage {
  constructor() { this._m = new Map(); }
  getItem(k) { return this._m.has(String(k)) ? this._m.get(String(k)) : null; }
  setItem(k, v) { this._m.set(String(k), String(v)); }
  removeItem(k) { this._m.delete(String(k)); }
  clear() { this._m.clear(); }
  key(i) { return Array.from(this._m.keys())[i] ?? null; }
  get length() { return this._m.size; }
}
globalThis.Storage = __Storage;
globalThis.localStorage = new __Storage();
globalThis.sessionStorage = new __Storage();

// FileReader as jsdom's (turbo-test#132, #139): a read reads the Blob's bytes, sets readyState
// LOADING at once, then (asynchronously) fires loadstart, progress (a non-empty Blob), load and
// loadend; abort() while LOADING fires abort and loadend instead. Listeners are kept in order
// (addEventListener deduplicated, on<type> handlers holding the slot of their first set).
if (typeof globalThis.FileReader === 'undefined') {
  const EVENTS = ['loadstart', 'progress', 'load', 'abort', 'error', 'loadend'];
  const invalidState = () => {
    const e = new globalThis.DOMException('The object is in an invalid state.', 'InvalidStateError');
    Object.defineProperty(e, 'code', { value: 11, configurable: true });
    return e;
  };
  const BOMS = [
    [[0xef, 0xbb, 0xbf], 'utf-8'],
    [[0xff, 0xfe], 'utf-16le'],
    [[0xfe, 0xff], 'utf-16be'],
  ];
  const bomMatches = (b, bytes) => bytes.every((v, i) => b[i] === v);
  const bomEncoding = (b) => {
    for (const [bytes, enc] of BOMS) if (bomMatches(b, bytes)) return enc;
    return null;
  };
  const labelEncoding = (label) => {
    if (label === undefined) return 'utf-8';
    try { return new globalThis.TextDecoder(String(label)).encoding; } catch { return 'utf-8'; }
  };
  // the encoding readAsText decodes with: a UTF-8 / UTF-16 BOM wins, then the label, then UTF-8
  const textOf = (b, label) => {
    const enc = bomEncoding(b) || labelEncoding(label);
    return new globalThis.TextDecoder(enc).decode(b);
  };
  // bytes[] -> FileReader#result for the four read kinds
  const binaryString = (bytes) => {
    let s = '';
    for (let i = 0; i < bytes.length; i += 8192) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
    return s;
  };
  const decodeReadResult = (bytes, blob, kind, label) => {
    if (kind === 'readAsDataURL') return 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + globalThis.__b64encodeBytes(bytes);
    if (kind === 'readAsArrayBuffer') return bytes.slice().buffer;
    if (kind === 'readAsBinaryString') return binaryString(bytes);
    return textOf(bytes, label);
  };
  globalThis.FileReader = class FileReader {
    #listeners = Object.create(null); // type -> [{ fn }] (an on<type> handler's entry has `on: true`)
    #gen = 0;                          // bumped by every read and abort; a stale read stops
    constructor() { this.result = null; this.error = null; this.readyState = 0; }
    addEventListener(type, fn) {
      if (fn == null) return;
      const list = this.#listeners[type] || (this.#listeners[type] = []);
      if (!list.some((l) => !l.on && l.fn === fn)) list.push({ fn });
    }
    removeEventListener(type, fn) {
      const list = this.#listeners[type];
      if (list) this.#listeners[type] = list.filter((l) => l.on || l.fn !== fn);
    }
    dispatchEvent(ev) {
      for (const l of (this.#listeners[ev.type] || []).slice()) {
        try {
          if (typeof l.fn === 'function') l.fn.call(this, ev);
          else if (l.fn && typeof l.fn.handleEvent === 'function') l.fn.handleEvent(ev);
        } catch (e) { console.error(e); } // reported, as jsdom's virtual console does
      }
      return !ev.defaultPrevented;
    }
    #handler(type) { const l = (this.#listeners[type] || []).find((x) => x.on); return l ? l.fn : null; }
    #setHandler(type, fn) {
      const list = this.#listeners[type] || (this.#listeners[type] = []);
      const at = list.findIndex((x) => x.on);
      if (typeof fn !== 'function' && (fn === null || typeof fn !== 'object')) { if (at >= 0) list.splice(at, 1); return; }
      if (at >= 0) list[at].fn = fn; else list.push({ fn, on: true });
    }
    #fire(type, loaded = 0, total = 0, lengthComputable = false) {
      const ev = new globalThis.Event(type);
      for (const [k, v] of [['target', this], ['currentTarget', this], ['loaded', loaded], ['total', total], ['lengthComputable', lengthComputable]])
        Object.defineProperty(ev, k, { value: v, configurable: true, enumerable: true });
      this.dispatchEvent(ev);
    }
    async #readBytes(blob) {
      // the Blob's bytes, not char codes of its decoded text (turbo-test#132)
      return typeof blob.bytes === 'function' ? await blob.bytes() : new Uint8Array(await blob.arrayBuffer());
    }
    #onReadError(gen, e) {
      if (gen !== this.#gen) return;
      this.readyState = 2; this.error = e;
      this.#fire('loadstart'); this.#fire('error'); this.#fire('loadend');
    }
    #onReadSuccess(gen, bytes, blob, kind, label) {
      if (gen !== this.#gen) return;
      this.#fire('loadstart');
      if (gen !== this.#gen) return;
      if (bytes.length) { this.#fire('progress', bytes.length, bytes.length, true); if (gen !== this.#gen) return; }
      this.result = decodeReadResult(bytes, blob, kind, label);
      this.readyState = 2;
      this.#fire('load');
      this.#fire('loadend');
    }
    #read(blob, kind, label) {
      if (blob === null || typeof blob !== 'object' || typeof blob.arrayBuffer !== 'function')
        throw new TypeError(`Failed to execute '${kind}' on 'FileReader': parameter 1 is not of type 'Blob'.`);
      if (this.readyState === 1) throw invalidState();
      const gen = ++this.#gen;
      this.readyState = 1;
      this.result = null;
      this.error = null;
      Promise.resolve().then(async () => {
        let bytes;
        try { bytes = await this.#readBytes(blob); }
        catch (e) { this.#onReadError(gen, e); return; }
        this.#onReadSuccess(gen, bytes, blob, kind, label);
      });
    }
    readAsArrayBuffer(blob) { this.#read(blob, 'readAsArrayBuffer'); }
    readAsBinaryString(blob) { this.#read(blob, 'readAsBinaryString'); }
    readAsDataURL(blob) { this.#read(blob, 'readAsDataURL'); }
    readAsText(blob, encoding = undefined) { this.#read(blob, 'readAsText', encoding); }
    abort() {
      if (this.readyState === 0 || this.readyState === 2) { this.result = null; return; }
      this.#gen++;
      this.readyState = 2;
      this.result = null;
      this.#fire('abort');
      this.#fire('loadend');
    }
    static #accessors = (() => {
      for (const type of EVENTS) Object.defineProperty(FileReader.prototype, 'on' + type, {
        get() { return this.#handler(type); },
        set(fn) { this.#setHandler(type, fn); },
        configurable: true, enumerable: true,
      });
    })();
  };
  ['EMPTY', 'LOADING', 'DONE'].forEach((k, i) => {
    Object.defineProperty(globalThis.FileReader, k, { value: i, enumerable: true });
    Object.defineProperty(globalThis.FileReader.prototype, k, { value: i, enumerable: true });
  });
}
// Minimal DOMMatrix/DOMPoint (SVG transform math used by charting libs at module load).
if (typeof globalThis.DOMMatrix === 'undefined') {
  class DOMMatrix {
    constructor(init) {
      this.a = 1; this.b = 0; this.c = 0; this.d = 1; this.e = 0; this.f = 0;
      this.m11 = 1; this.m12 = 0; this.m13 = 0; this.m14 = 0;
      this.m21 = 0; this.m22 = 1; this.m23 = 0; this.m24 = 0;
      this.m31 = 0; this.m32 = 0; this.m33 = 1; this.m34 = 0;
      this.m41 = 0; this.m42 = 0; this.m43 = 0; this.m44 = 1;
      this.is2D = true; this.isIdentity = true;
      if (Array.isArray(init) && init.length >= 6) { [this.a, this.b, this.c, this.d, this.e, this.f] = init; this.m11 = this.a; this.m12 = this.b; this.m21 = this.c; this.m22 = this.d; this.m41 = this.e; this.m42 = this.f; }
    }
    multiply() { return new DOMMatrix(); }
    multiplySelf() { return this; }
    preMultiplySelf() { return this; }
    translate() { return new DOMMatrix(); }
    translateSelf() { return this; }
    scale() { return new DOMMatrix(); }
    scaleSelf() { return this; }
    scale3d() { return new DOMMatrix(); }
    rotate() { return new DOMMatrix(); }
    rotateSelf() { return this; }
    rotateFromVector() { return new DOMMatrix(); }
    skewX() { return new DOMMatrix(); }
    skewY() { return new DOMMatrix(); }
    inverse() { return new DOMMatrix(); }
    invertSelf() { return this; }
    flipX() { return new DOMMatrix(); }
    flipY() { return new DOMMatrix(); }
    transformPoint(p) { return new globalThis.DOMPoint(p && p.x, p && p.y, p && p.z, p && p.w); }
    toFloat32Array() { return new Float32Array(16); }
    toFloat64Array() { return new Float64Array(16); }
    toString() { return 'matrix(1, 0, 0, 1, 0, 0)'; }
    static fromMatrix() { return new DOMMatrix(); }
    static fromFloat32Array() { return new DOMMatrix(); }
    static fromFloat64Array() { return new DOMMatrix(); }
  }
  globalThis.DOMMatrix = DOMMatrix;
  globalThis.DOMMatrixReadOnly = DOMMatrix;
  globalThis.WebKitCSSMatrix = DOMMatrix;
  globalThis.SVGMatrix = DOMMatrix;
}
// Web Streams. This used to be a stub whose reader always resolved `{ done: true }` — the globals
// existed (so `class X extends TransformStream` loaded) but any code that actually CONSUMED a
// stream (fetch body / SSE parsing) silently saw an empty stream. This is a working implementation:
// chunk queue + pull backpressure, reader/writer locking, cancel/abort propagation, tee, async
// iteration, pipeTo/pipeThrough. Not spec-exhaustive (no BYOB readers, no queuing strategy `size`).
// A host that already provides native streams (napi/Node) still wins via the guard.
if (typeof globalThis.DOMPoint === 'undefined') {
  class DOMPoint {
    constructor(x, y, z, w) { this.x = x || 0; this.y = y || 0; this.z = z || 0; this.w = w == null ? 1 : w; }
    matrixTransform() { return this; }
    static fromPoint(p) { return new DOMPoint(p && p.x, p && p.y, p && p.z, p && p.w); }
  }
  globalThis.DOMPoint = DOMPoint;
  globalThis.DOMPointReadOnly = DOMPoint;
}
// FileReader is defined above EventTarget; make it one (turbo-test#139)
if (globalThis.FileReader && !(globalThis.FileReader.prototype instanceof globalThis.EventTarget)) {
  Object.setPrototypeOf(globalThis.FileReader.prototype, globalThis.EventTarget.prototype);
  Object.setPrototypeOf(globalThis.FileReader, globalThis.EventTarget);
}
if (typeof globalThis.MutationObserver === 'undefined') {
  globalThis.MutationObserver = class MutationObserver { constructor(cb) { this._cb = cb; } observe() {} disconnect() {} takeRecords() { return []; } };
}
if (typeof globalThis.requestAnimationFrame === 'undefined') {
  globalThis.requestAnimationFrame = (cb) => globalThis.setTimeout(() => cb(0), 0);
  globalThis.cancelAnimationFrame = (id) => globalThis.clearTimeout(id);
}

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
