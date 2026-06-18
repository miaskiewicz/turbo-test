(function(){
  var g = globalThis;
  if (typeof g.navigator === 'undefined') g.navigator = { userAgent: 'turbo-test', platform: 'rust', language: 'en-US', languages: ['en-US'], clipboard: {}, maxTouchPoints: 0 };
  if (typeof g.getComputedStyle === 'undefined') g.getComputedStyle = function(){ return { getPropertyValue: function(){ return ''; } }; };
  if (typeof g.requestAnimationFrame === 'undefined') g.requestAnimationFrame = function(cb){ return setTimeout(function(){ cb(Date.now()); }, 0); };
  if (typeof g.cancelAnimationFrame === 'undefined') g.cancelAnimationFrame = function(id){ clearTimeout(id); };
  if (typeof g.matchMedia === 'undefined') g.matchMedia = function(q){ return { matches:false, media:q, addListener:function(){}, removeListener:function(){}, addEventListener:function(){}, removeEventListener:function(){}, dispatchEvent:function(){return false;} }; };
  if (typeof g.scrollTo === 'undefined') g.scrollTo = function(){};
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
  d.createComment = function(data){ return d.createTextNode(data); };
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
    var protoFor = {};
    var defType = { HTMLInputElement:'text', HTMLButtonElement:'submit' };
    ['HTMLInputElement','HTMLTextAreaElement','HTMLSelectElement','HTMLOptionElement','HTMLButtonElement'].forEach(function(n){ if (typeof g[n] !== 'function') g[n] = function(){}; var p = Object.create(baseProto); try {
      Object.defineProperty(p, 'value', valDesc); Object.defineProperty(p, 'checked', checkedDesc); Object.defineProperty(p, 'form', formDesc);
      if (defType[n]) Object.defineProperty(p, 'type', mkTypeDesc(defType[n]));
      if (n === 'HTMLInputElement' || n === 'HTMLTextAreaElement') {
        Object.defineProperty(p, 'selectionStart', selStartDesc); Object.defineProperty(p, 'selectionEnd', selEndDesc); Object.defineProperty(p, 'selectionDirection', selDirDesc);
        p.setSelectionRange = function(s, e, dir){ this.__selStart = s; this.__selEnd = e; this.__selDir = dir || 'none'; };
        p.select = function(){ this.__selStart = 0; this.__selEnd = String(this.value||'').length; };
        p.setRangeText = function(){};
      }
    } catch(e){} g[n].prototype = p; protoFor[n] = p; });
    var CTRL = { input:'HTMLInputElement', textarea:'HTMLTextAreaElement', select:'HTMLSelectElement', option:'HTMLOptionElement', button:'HTMLButtonElement' };
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
