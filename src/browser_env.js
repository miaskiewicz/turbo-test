(function(){
  var g = globalThis;
  if (typeof g.navigator === 'undefined') g.navigator = { userAgent: 'turbo-test', platform: 'rust', language: 'en-US', languages: ['en-US'], clipboard: {}, maxTouchPoints: 0 };
  // getComputedStyle reflects the element's inline style object (React writes el.style.X), so
  // jest-dom toHaveStyle (computedStyle[prop] / getPropertyValue(prop)) and toBeVisible (display/
  // visibility/opacity) read real values. No cascade — inline styles only, which covers these tests.
  if (typeof g.getComputedStyle === 'undefined') g.getComputedStyle = function(el){
    var st = el && el.style;
    if (st) {
      var camel = function(p){ return p.replace(/-([a-z])/g, function(_,c){ return c.toUpperCase(); }); };
      return {
        getPropertyValue: function(p){ var v = st.getPropertyValue ? st.getPropertyValue(p) : st[p]; if (v) return v; var c = st[camel(p)]; return c == null ? '' : String(c); },
        get display(){ return st.display || ''; }, get visibility(){ return st.visibility || ''; }, get opacity(){ return st.opacity || ''; },
        length: 0
      };
    }
    return { getPropertyValue: function(){ return ''; }, display:'', visibility:'', opacity:'', length:0 };
  };
  if (typeof g.requestAnimationFrame === 'undefined') g.requestAnimationFrame = function(cb){ return setTimeout(function(){ cb(Date.now()); }, 0); };
  if (typeof g.cancelAnimationFrame === 'undefined') g.cancelAnimationFrame = function(id){ clearTimeout(id); };
  if (typeof g.matchMedia === 'undefined') g.matchMedia = function(q){ return { matches:false, media:q, addListener:function(){}, removeListener:function(){}, addEventListener:function(){}, removeEventListener:function(){}, dispatchEvent:function(){return false;} }; };
  if (typeof g.scrollTo === 'undefined') g.scrollTo = function(){};
  if (typeof g.location === 'undefined' || g.location == null) {
    g.location = { href: 'http://localhost/', origin: 'http://localhost', protocol: 'http:', host: 'localhost', hostname: 'localhost', port: '', pathname: '/', search: '', hash: '', assign: function(){}, replace: function(){}, reload: function(){}, toString: function(){ return this.href; } };
  }
  // DOM interface constructors (for `instanceof` / global presence). Stubs; identity not enforced.
  // Real Event base class (dispatch in browser_env.rs reads type/bubbles/defaultPrevented/__stop*).
  if (!g.__ttEvent) {
    function Event(type, init){ init = init || {}; this.type = type; this.bubbles = !!init.bubbles; this.cancelable = !!init.cancelable; this.composed = !!init.composed; this.defaultPrevented = false; this.target = null; this.currentTarget = null; this.__stop = false; this.__stopImmediate = false; this.eventPhase = 0; this.timeStamp = Date.now(); this.isTrusted = false; this.detail = init.detail; }
    Event.prototype.preventDefault = function(){ if (this.cancelable) this.defaultPrevented = true; };
    Event.prototype.stopPropagation = function(){ this.__stop = true; };
    Event.prototype.stopImmediatePropagation = function(){ this.__stop = true; this.__stopImmediate = true; };
    Event.prototype.initEvent = function(type, bubbles, cancelable){ this.type = type; this.bubbles = !!bubbles; this.cancelable = !!cancelable; };
    Event.NONE = 0; Event.CAPTURING_PHASE = 1; Event.AT_TARGET = 2; Event.BUBBLING_PHASE = 3;
    g.Event = Event;
    function mkSub(extra){ return function(type, init){ init = init || {}; Event.call(this, type, init); if (extra) extra(this, init); for (var k in init){ if (!(k in this)) this[k] = init[k]; } }; }
    function sub(name, extra){ var C = mkSub(extra); C.prototype = Object.create(Event.prototype); C.prototype.constructor = C; g[name] = C; }
    var keyExtra = function(self, init){ self.key = init.key||''; self.code = init.code||''; self.keyCode = init.keyCode||0; self.which = init.which||init.keyCode||0; self.altKey=!!init.altKey; self.ctrlKey=!!init.ctrlKey; self.metaKey=!!init.metaKey; self.shiftKey=!!init.shiftKey; self.repeat=!!init.repeat; };
    var mouseExtra = function(self, init){ self.button=init.button||0; self.buttons=init.buttons||0; self.clientX=init.clientX||0; self.clientY=init.clientY||0; self.pageX=init.pageX||0; self.pageY=init.pageY||0; self.altKey=!!init.altKey; self.ctrlKey=!!init.ctrlKey; self.metaKey=!!init.metaKey; self.shiftKey=!!init.shiftKey; self.relatedTarget=init.relatedTarget||null; };
    sub('CustomEvent', function(self, init){ self.detail = init.detail; });
    sub('UIEvent'); sub('MouseEvent', mouseExtra); sub('PointerEvent', mouseExtra); sub('KeyboardEvent', keyExtra);
    sub('InputEvent', function(self, init){ self.data = init.data; self.inputType = init.inputType||''; });
    sub('FocusEvent', function(self, init){ self.relatedTarget = init.relatedTarget||null; });
    sub('CompositionEvent'); sub('WheelEvent', mouseExtra); sub('DragEvent', mouseExtra); sub('TouchEvent'); sub('ClipboardEvent');
    g.document.createEvent = function(){ return new Event('', {}); };
    g.__ttEvent = true;
  }
  var ctors = ['Node','Element','HTMLElement','HTMLDivElement','HTMLInputElement','HTMLButtonElement','HTMLAnchorElement','HTMLSelectElement','HTMLTextAreaElement','HTMLFormElement','HTMLImageElement','HTMLLabelElement','HTMLOptionElement','HTMLUListElement','HTMLLIElement','HTMLSpanElement','HTMLParagraphElement','HTMLHeadingElement','HTMLTableElement','HTMLIFrameElement','HTMLCanvasElement','HTMLStyleElement','HTMLScriptElement','HTMLDocument','Document','DocumentFragment','ShadowRoot','Text','Comment','SVGElement','SVGSVGElement','DOMParser','EventTarget','AbortController','AbortSignal','DOMException',
    'UIEvent','MouseEvent','KeyboardEvent','FocusEvent','InputEvent','TouchEvent','PointerEvent','WheelEvent','DragEvent','ClipboardEvent','AnimationEvent','TransitionEvent','MessageEvent','ProgressEvent','CompositionEvent','PopStateEvent','HashChangeEvent','StorageEvent','ErrorEvent','CloseEvent'];
  ctors.forEach(function(n){ if (typeof g[n] === "undefined") { var f = function(){}; f.prototype = {}; try { Object.defineProperty(f, "name", { value: n, configurable: true }); } catch(e){} g[n] = f; } });
  // make `node instanceof HTMLElement/Element/Node/...` work for native DOM nodes (jest-dom's
  // checkHtmlElement + many libs rely on it) via Symbol.hasInstance keyed on nodeType.
  function iface(name, pred){ var f = g[name] || function(){}; try { Object.defineProperty(f, Symbol.hasInstance, { configurable: true, value: pred }); } catch(e){} g[name] = f; }
  var isNode = function(o){ return o != null && typeof o === 'object' && typeof o.nodeType === 'number'; };
  iface('Node', isNode);
  iface('Element', function(o){ return isNode(o) && o.nodeType === 1; });
  iface('HTMLElement', function(o){ return isNode(o) && o.nodeType === 1; });
  iface('SVGElement', function(o){ return isNode(o) && o.nodeType === 1 && String(o.namespaceURI||'').indexOf('svg') >= 0; });
  iface('Text', function(o){ return isNode(o) && o.nodeType === 3; });
  iface('Comment', function(o){ return isNode(o) && o.nodeType === 8; });
  iface('DocumentFragment', function(o){ return isNode(o) && o.nodeType === 11; });
  iface('HTMLInputElement', function(o){ return isNode(o) && o.nodeType === 1 && String(o.tagName).toUpperCase() === 'INPUT'; });
  iface('HTMLTextAreaElement', function(o){ return isNode(o) && o.nodeType === 1 && String(o.tagName).toUpperCase() === 'TEXTAREA'; });
  iface('HTMLSelectElement', function(o){ return isNode(o) && o.nodeType === 1 && String(o.tagName).toUpperCase() === 'SELECT'; });
  // window-level event listeners (no-op until native events land).
  if (typeof g.addEventListener === 'undefined') g.addEventListener = function(){};
  if (typeof g.removeEventListener === 'undefined') g.removeEventListener = function(){};
  if (typeof g.dispatchEvent === 'undefined') g.dispatchEvent = function(){ return true; };
  // document extras (pure-JS shims over the native tree).
  var d = g.document;
  d.defaultView = g;
  // React's isEventSupported('input') checks `'oninput' in document`; make it true (+ keep
  // documentMode ABSENT) so React uses the modern input/change path, not the IE change polyfill —
  // otherwise fireEvent.change never fires onChange.
  d.oninput = null; d.onchange = null; d.onclick = null; d.onkeydown = null; d.onkeyup = null;
  if (!d.head) { try { d.head = d.createElement('head'); if (d.documentElement) d.documentElement.appendChild(d.head); } catch(e){} }
  d.createElementNS = function(ns, tag){ return d.createElement(tag); };
  d.createDocumentFragment = function(){ return d.createElement('#document-fragment'); };
  d.addEventListener = function(){};
  d.removeEventListener = function(){};
  d.dispatchEvent = function(){ return true; };
  // CSSOM shim (<style>.sheet) + form-control value/checked. value/checked are defined BOTH as an
  // OWN accessor on each control element (testing-library's setNativeValue reads
  // getOwnPropertyDescriptor(element)) AND on the interface .prototype (React's value-tracker reads
  // node.constructor.prototype). No setPrototypeOf — avoids touching the native proto chain.
  (function(){
    var sheets = [];
    var mkSheet = function(el){ var rules = []; return { ownerNode: el, cssRules: rules, get rules(){ return rules; }, insertRule: function(rule, index){ var i = index == null ? rules.length : index; rules.splice(i, 0, { cssText: String(rule), selectorText: '' }); return i; }, deleteRule: function(i){ rules.splice(i, 1); }, replaceSync: function(){}, replace: function(){ return Promise.resolve(); } }; };
    var orig = d.createElement.bind(d);
    // value/checked live ONLY on the interface .prototype (NOT own — React's value-tracker bails on
    // node.hasOwnProperty('value')). Each control's actual proto is set to that interface prototype
    // so getPrototypeOf(el) === el.constructor.prototype has the descriptor (React + testing-library).
    var valDesc = { configurable: true, get: function(){ var v = this.getAttribute('value'); return v == null ? '' : v; }, set: function(v){ this.setAttribute('value', v == null ? '' : String(v)); } };
    var checkedDesc = { configurable: true, get: function(){ return this.__checked === undefined ? this.hasAttribute('checked') : !!this.__checked; }, set: function(v){ this.__checked = !!v; } };
    // `type` defaults to 'text' for <input> — React's isTextInputElement keys on it to pick the
    // input/change handling path; undefined would route changes to the select/checkbox path.
    var mkTypeDesc = function(def){ return { configurable: true, get: function(){ return (this.getAttribute('type') || def).toLowerCase(); }, set: function(v){ this.setAttribute('type', v); } }; };
    // `form` → the nearest ancestor <form> (userEvent fires submit on a submit-button's form).
    var formDesc = { configurable: true, get: function(){ return this.closest ? this.closest('form') : null; } };
    // text-cursor selection (userEvent.type inserts at selectionStart..selectionEnd; undefined -> NaN
    // -> nothing gets typed). Default the cursor to the end of the value.
    var selStartDesc = { configurable: true, get: function(){ return this.__selStart == null ? String(this.value||'').length : this.__selStart; }, set: function(v){ this.__selStart = v; } };
    var selEndDesc = { configurable: true, get: function(){ return this.__selEnd == null ? String(this.value||'').length : this.__selEnd; }, set: function(v){ this.__selEnd = v; } };
    var selDirDesc = { configurable: true, get: function(){ return this.__selDir || 'none'; }, set: function(v){ this.__selDir = v; } };
    var baseProto = Object.getPrototypeOf(orig('span'));
    // classList on the shared element prototype (every element inherits it), backed by className.
    if (baseProto && !Object.getOwnPropertyDescriptor(baseProto, 'classList')) {
      try { Object.defineProperty(baseProto, 'classList', { configurable: true, get: function(){
        var el = this;
        var parse = function(){ return String(el.className || '').split(/\s+/).filter(Boolean); };
        var write = function(a){ el.className = a.join(' '); };
        var dt = {
          add: function(){ var a = parse(); for (var i=0;i<arguments.length;i++) if (a.indexOf(arguments[i])<0) a.push(arguments[i]); write(a); },
          remove: function(){ var a = parse(); for (var i=0;i<arguments.length;i++){ var x=a.indexOf(arguments[i]); if (x>=0) a.splice(x,1); } write(a); },
          toggle: function(c, force){ var a = parse(); var has = a.indexOf(c)>=0; var on = force === undefined ? !has : !!force; if (on && !has) a.push(c); else if (!on && has) a.splice(a.indexOf(c),1); write(a); return on; },
          contains: function(c){ return parse().indexOf(c)>=0; },
          replace: function(o,n){ var a = parse(); var x = a.indexOf(o); if (x<0) return false; a[x]=n; write(a); return true; },
          item: function(i){ return parse()[i] || null; },
          forEach: function(cb, thisArg){ parse().forEach(cb, thisArg); },
          toString: function(){ return String(el.className || ''); }
        };
        Object.defineProperty(dt, 'length', { get: function(){ return parse().length; } });
        Object.defineProperty(dt, 'value', { get: function(){ return String(el.className || ''); } });
        return dt;
      } }); } catch(e){}
    }
    var protoFor = {};
    var defType = { HTMLInputElement:'text', HTMLButtonElement:'submit' };
    ['HTMLInputElement','HTMLTextAreaElement','HTMLSelectElement','HTMLOptionElement','HTMLButtonElement','HTMLLabelElement'].forEach(function(n){ if (typeof g[n] !== 'function') g[n] = function(){}; var p = Object.create(baseProto); try {
      Object.defineProperty(p, 'value', valDesc); Object.defineProperty(p, 'checked', checkedDesc); Object.defineProperty(p, 'form', formDesc);
      if (defType[n]) Object.defineProperty(p, 'type', mkTypeDesc(defType[n]));
      if (n === 'HTMLInputElement' || n === 'HTMLTextAreaElement') {
        Object.defineProperty(p, 'selectionStart', selStartDesc); Object.defineProperty(p, 'selectionEnd', selEndDesc); Object.defineProperty(p, 'selectionDirection', selDirDesc);
        p.setSelectionRange = function(s, e, dir){ this.__selStart = s; this.__selEnd = e; this.__selDir = dir || 'none'; };
        p.select = function(){ this.__selStart = 0; this.__selEnd = String(this.value||'').length; };
        p.setRangeText = function(){};
      }
    } catch(e){} g[n].prototype = p; protoFor[n] = p; });
    // Descendant <option>s. Prefer querySelectorAll (document order); fall back to a `children` walk
    // when it returns empty — that happens on a DETACHED subtree (qsa matches only connected nodes),
    // which is exactly when React-DOM's updateOptions reads select.options DURING commit (the subtree
    // is still detached then). Without the fallback a controlled <select> never marks its option.
    var optionsOf = function(sel){
      var a = []; var list = sel.querySelectorAll('option');
      for (var i=0;i<list.length;i++) a.push(list[i]);
      if (!a.length) (function walk(n){ var ch = n.children; for (var j=0;j<ch.length;j++){ var c = ch[j]; var tg = String(c.tagName).toUpperCase(); if (tg === 'OPTION') a.push(c); else if (tg === 'OPTGROUP') walk(c); } })(sel);
      a.item = function(i){ return this[i] || null; }; return a;
    };
    var ownerSelect = function(n){ var p = n.parentNode; while (p && p.nodeType === 1){ if (String(p.tagName).toUpperCase() === 'SELECT') return p; p = p.parentNode; } return null; };
    // <option>: `value` falls back to text content (HTML spec), `selected`/`defaultSelected` track
    // the live + attribute state. React-DOM's updateOptions reads option.value and writes
    // option.selected for every option of a controlled <select>.
    (function(){
      var op = protoFor.HTMLOptionElement;
      Object.defineProperty(op, 'value', { configurable: true, get: function(){ var v = this.getAttribute('value'); return v == null ? (this.textContent || '') : v; }, set: function(v){ this.setAttribute('value', v == null ? '' : String(v)); } });
      // Setting selected=true on a single (non-multiple) <select> deselects its siblings — the
      // browser enforces the single-selection invariant. Without this, an option marked during the
      // detached commit stays selected after userEvent.selectOptions marks another, and select.value
      // (first selected) returns the stale one.
      Object.defineProperty(op, 'selected', { configurable: true, get: function(){ return this.__selected === undefined ? this.hasAttribute('selected') : !!this.__selected; }, set: function(v){ v = !!v; this.__selected = v; if (v){ var s = ownerSelect(this); if (s && !s.multiple){ var os = optionsOf(s); for (var i=0;i<os.length;i++) if (os[i] !== this) os[i].__selected = false; } } } });
      Object.defineProperty(op, 'defaultSelected', { configurable: true, get: function(){ return this.hasAttribute('selected'); }, set: function(v){ if (v) this.setAttribute('selected',''); else this.removeAttribute('selected'); } });
      Object.defineProperty(op, 'disabled', { configurable: true, get: function(){ return this.hasAttribute('disabled'); }, set: function(v){ if (v) this.setAttribute('disabled',''); else this.removeAttribute('disabled'); } });
      Object.defineProperty(op, 'text', { configurable: true, get: function(){ return this.textContent || ''; }, set: function(v){ this.textContent = v; } });
    })();
    // <select>: `options` (HTMLOptionsCollection — the crash: React-DOM reads node.options.length),
    // plus `value`/`selectedIndex` derived from the options' selected state.
    (function(){
      var sp = protoFor.HTMLSelectElement;
      var opts = optionsOf;
      Object.defineProperty(sp, 'options', { configurable: true, get: function(){ return opts(this); } });
      Object.defineProperty(sp, 'selectedOptions', { configurable: true, get: function(){ return opts(this).filter(function(o){ return o.selected; }); } });
      Object.defineProperty(sp, 'selectedIndex', { configurable: true,
        get: function(){ var o = opts(this); for (var i=0;i<o.length;i++) if (o[i].selected) return i; return o.length ? -1 : -1; },
        set: function(idx){ var o = opts(this); for (var i=0;i<o.length;i++) o[i].selected = (i === idx); } });
      Object.defineProperty(sp, 'value', { configurable: true,
        get: function(){ var o = opts(this); for (var i=0;i<o.length;i++) if (o[i].selected) return o[i].value; return o.length ? o[0].value : ''; },
        set: function(v){ var o = opts(this); var s = String(v); var hit = false; for (var i=0;i<o.length;i++){ var m = (o[i].value === s); o[i].selected = m; if (m) hit = true; } if (!hit) for (var j=0;j<o.length;j++) o[j].selected = false; } });
    })();
    // <label>: htmlFor + control resolution. testing-library's getByLabelText filters
    // querySelectorAll('label') by `label.control === element`, so without `control` NO label ever
    // associates and every form field fails with "no form control was found associated to that label".
    (function(){
      var lp = protoFor.HTMLLabelElement;
      var labelable = function(el){ if (!el || el.nodeType !== 1) return false; var tg = String(el.tagName).toUpperCase(); if (tg === 'INPUT') return (el.getAttribute('type') || 'text').toLowerCase() !== 'hidden'; return tg === 'BUTTON' || tg === 'SELECT' || tg === 'TEXTAREA' || tg === 'METER' || tg === 'OUTPUT' || tg === 'PROGRESS'; };
      Object.defineProperty(lp, 'htmlFor', { configurable: true, get: function(){ return this.getAttribute('for') || ''; }, set: function(v){ this.setAttribute('for', v == null ? '' : String(v)); } });
      Object.defineProperty(lp, 'control', { configurable: true, get: function(){
        var f = this.getAttribute('for');
        if (f) { var el = this.ownerDocument.getElementById(f); return labelable(el) ? el : null; }
        var list = this.querySelectorAll('input, button, select, textarea, meter, output, progress');
        for (var i=0;i<list.length;i++) if (labelable(list[i])) return list[i];
        return null;
      } });
    })();
    var CTRL = { input:'HTMLInputElement', textarea:'HTMLTextAreaElement', select:'HTMLSelectElement', option:'HTMLOptionElement', button:'HTMLButtonElement', label:'HTMLLabelElement' };
    d.createElement = function(tag){
      var el = orig(tag); var t = String(tag).toLowerCase();
      try {
        if (t === 'style' && !el.sheet) { var s = mkSheet(el); Object.defineProperty(el, 'sheet', { configurable: true, get: function(){ return s; } }); sheets.push(s); }
        if (CTRL[t]) {
          Object.setPrototypeOf(el, protoFor[CTRL[t]]);
          // define value/checked as OWN props too: React's value-tracker bails when
          // node.hasOwnProperty('value') -> getInstIfValueChanged returns true -> onChange fires on
          // EVERY input event (so userEvent.type per-char onChange works), while testing-library
          // still finds the own setter.
          if (t === 'input' || t === 'textarea') { Object.defineProperty(el, 'value', valDesc); Object.defineProperty(el, 'checked', checkedDesc); }
        }
      } catch(e){}
      return el;
    };
    if (!d.styleSheets) { try { Object.defineProperty(d, 'styleSheets', { configurable: true, get: function(){ return sheets; } }); } catch(e){} }
  })();
  d.createRange = function(){ return { setStart:function(){}, setEnd:function(){}, selectNodeContents:function(){}, collapse:function(){}, getClientRects:function(){return [];}, getBoundingClientRect:function(){return {x:0,y:0,top:0,left:0,right:0,bottom:0,width:0,height:0};}, createContextualFragment:function(html){ var f=d.createDocumentFragment(); var t=d.createElement("div"); t.innerHTML=html; while(t.firstChild) f.appendChild(t.firstChild); return f; }, cloneRange:function(){return d.createRange();}, detach:function(){}, commonAncestorContainer: d.body }; };
  if (!d.getRootNode) d.getRootNode = function(){ return d; };
  if (!d.getSelection) d.getSelection = function(){ return { removeAllRanges:function(){}, addRange:function(){}, getRangeAt:function(){return d.createRange();}, rangeCount:0, toString:function(){return "";} }; };
  if (!g.getSelection) g.getSelection = d.getSelection;
})();
