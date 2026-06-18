//! All-Rust DOM (P3) — binds turbo-dom's pure-Rust `rtdom` (the `turbo-dom-parser` crate, built with
//! `rust-runtime`) to V8 via native rusty-v8 callbacks, so tests get `window`/`document`/Element with
//! NO JS DOM (no `installGlobals`, no `.node` parser, no node_modules turbo-dom).
//!
//! Model: the DOM lives in Rust as one `Tree` per context (handle-based — each node is a `u32`
//! `Handle`). A JS DOM node is a V8 object with the handle in internal field 0; methods/accessors
//! are native callbacks that read the handle, call `Tree`, and wrap result handles back into node
//! objects. A `handle → V8 object` cache preserves identity (`el === el`).
//!
//! This is the P3.2 vertical slice: document/createElement/createTextNode, appendChild/insertBefore/
//! removeChild, get/setAttribute, id/className/tagName/textContent, querySelector(All)/getElementById.
//! Events, cssom/style, classList, forms etc. grow in P3.3. Gated by `TURBO_RUST_DOM`.

use std::cell::RefCell;
use std::collections::HashMap;

use turbo_dom_parser::rtdom::node_ref::DocumentExt;
use turbo_dom_parser::rtdom::tree::{Handle, Tree};
use turbo_dom_parser::rtdom::NodeRef;

pub fn enabled() -> bool {
    std::env::var("TURBO_RUST_DOM").map(|v| !v.is_empty() && v != "0").unwrap_or(false)
}

/// Whether to RECORD missing-member accesses (debug surface map). The interceptor itself is always
/// installed (for graceful degradation — an unimplemented member returns undefined instead of
/// crashing the file); recording is the only extra cost and is gated here so production pays nothing
/// but a cached bool check + a small fall-through. Cached once (no per-access getenv).
pub fn log_enabled() -> bool {
    use std::sync::OnceLock;
    static LOG: OnceLock<bool> = OnceLock::new();
    *LOG.get_or_init(|| std::env::var("TURBO_RUST_DOM_LOG").map(|v| !v.is_empty() && v != "0").unwrap_or(false))
}

/// Names the native DOM binds (template methods/accessors + document own methods) + JS internals —
/// the interceptor falls through for these and only records the REST as "missing".
const KNOWN: &[&str] = &[
    // element methods + accessors
    "appendChild", "removeChild", "insertBefore", "setAttribute", "getAttribute", "hasAttribute",
    "removeAttribute", "querySelector", "querySelectorAll", "tagName", "parentNode", "firstChild",
    "nextSibling", "textContent", "id", "className",
    "style", "oninput", "onchange", "onclick",
    "nodeType", "nodeName", "childNodes", "ownerDocument",
    "addEventListener", "removeEventListener", "dispatchEvent",
    "matches", "contains",
    "innerHTML", "outerHTML", "children", "parentElement", "firstElementChild", "namespaceURI",
    "value", "append", "prepend", "remove", "focus", "blur", "click", "scrollIntoView",
    "getBoundingClientRect", "createElementNS", "createDocumentFragment", "createComment",
    // document own methods/props
    "createElement", "createTextNode", "getElementById", "body", "documentElement",
    // JS internals V8 / libs probe constantly — never DOM
    "then", "catch", "finally", "constructor", "prototype", "toString", "valueOf", "toJSON",
    "length", "name", "call", "apply", "bind", "hasOwnProperty", "nodeName",
];

thread_local! {
    /// debug logger: unknown property name → access count.
    static MISSING: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
}

/// Interceptor getter (debug only): fall through for known/symbol names; record + return undefined
/// for anything unimplemented.
fn missing_getter(scope: &mut v8::PinScope, name: v8::Local<v8::Name>, _args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) -> v8::Intercepted {
    let key = name.to_rust_string_lossy(scope);
    // symbols (Symbol.toPrimitive, Symbol.iterator, …) + known names → fall through.
    if key.starts_with("Symbol(") || KNOWN.contains(&key.as_str()) {
        return v8::Intercepted::kNo;
    }
    if log_enabled() {
        MISSING.with(|m| *m.borrow_mut().entry(key).or_insert(0) += 1);
    }
    // graceful degradation: an unimplemented member reads as undefined (no crash).
    rv.set(v8::undefined(scope).into());
    v8::Intercepted::kYes
}

/// Passthrough setter interceptor: NEVER intercept (kNo) → JS property sets create normal own
/// properties. Required so `document.x = …` shims AND React's expando writes on DOM nodes
/// (`node.__reactProps$…`, `_reactListening…`) persist instead of being swallowed by the
/// getter-only interceptor.
fn passthrough_setter(_scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, _value: v8::Local<v8::Value>, _args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) -> v8::Intercepted {
    v8::Intercepted::kNo
}

/// Print + clear the accumulated missing-access log (call between files when debugging).
pub fn dump_missing() {
    let mut items: Vec<(String, u64)> = MISSING.with(|m| m.borrow_mut().drain().collect());
    if items.is_empty() {
        return;
    }
    items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    eprintln!("[rust-dom] {} missing DOM members (name×accesses):", items.len());
    for (name, n) in &items {
        eprintln!("[rust-dom]   {name} ×{n}");
    }
}

struct DomState {
    tree: Tree,
    /// handle → JS node object, for identity (`el === el`).
    cache: HashMap<Handle, v8::Global<v8::Object>>,
    /// shared element object template (internal_field_count = 1).
    el_template: v8::Global<v8::ObjectTemplate>,
}

thread_local! {
    static DOM: RefCell<Option<DomState>> = const { RefCell::new(None) };
}

/// Read a node object's handle from internal field 0.
fn handle_of(scope: &mut v8::PinScope, obj: v8::Local<v8::Object>) -> Option<Handle> {
    let f = obj.get_internal_field(scope, 0)?;
    let v = v8::Local::<v8::Value>::try_from(f).ok()?;
    v.uint32_value(scope).map(|x| x as Handle)
}

/// Wrap a handle as a JS node object (cached for identity).
fn wrap<'s>(scope: &mut v8::PinScope<'s, '_>, handle: Handle) -> v8::Local<'s, v8::Object> {
    if let Some(g) = DOM.with(|d| d.borrow().as_ref().and_then(|s| s.cache.get(&handle).cloned())) {
        return v8::Local::new(scope, &g);
    }
    let tmpl_g = DOM.with(|d| d.borrow().as_ref().unwrap().el_template.clone());
    let tmpl = v8::Local::new(scope, &tmpl_g);
    let obj = tmpl.new_instance(scope).unwrap();
    let h = v8::Integer::new_from_unsigned(scope, handle);
    obj.set_internal_field(0, h.into());
    let g = v8::Global::new(scope, obj);
    DOM.with(|d| {
        if let Some(s) = d.borrow_mut().as_mut() {
            s.cache.insert(handle, g);
        }
    });
    obj
}

/// Wrap `Option<Handle>` as object-or-null.
fn wrap_opt<'s>(scope: &mut v8::PinScope<'s, '_>, h: Option<Handle>) -> v8::Local<'s, v8::Value> {
    match h {
        Some(h) => wrap(scope, h).into(),
        None => v8::null(scope).into(),
    }
}

fn arg_str(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments, i: i32) -> String {
    args.get(i).to_rust_string_lossy(scope)
}

// ---- with-tree helpers (borrow the thread-local Tree) ----------------------------------------

fn with_tree<R>(f: impl FnOnce(&Tree) -> R) -> Option<R> {
    DOM.with(|d| d.borrow().as_ref().map(|s| f(&s.tree)))
}
fn with_tree_mut<R>(f: impl FnOnce(&mut Tree) -> R) -> Option<R> {
    DOM.with(|d| d.borrow_mut().as_mut().map(|s| f(&mut s.tree)))
}

// ---- element methods -------------------------------------------------------------------------

fn el_append_child(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(parent) = handle_of(scope, args.this()) else { return };
    let Some(child_obj) = args.get(0).to_object(scope) else { return };
    let Some(child) = handle_of(scope, child_obj) else { return };
    with_tree_mut(|t| t.append_child(parent, child));
    rv.set(args.get(0));
}

fn el_remove_child(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(parent) = handle_of(scope, args.this()) else { return };
    let Some(child_obj) = args.get(0).to_object(scope) else { return };
    let Some(child) = handle_of(scope, child_obj) else { return };
    with_tree_mut(|t| t.remove_child(parent, child));
    rv.set(args.get(0));
}

fn el_insert_before(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(parent) = handle_of(scope, args.this()) else { return };
    let Some(child_obj) = args.get(0).to_object(scope) else { return };
    let Some(child) = handle_of(scope, child_obj) else { return };
    let reference = args.get(1).to_object(scope).and_then(|o| handle_of(scope, o));
    with_tree_mut(|t| t.insert_before(parent, child, reference));
    rv.set(args.get(0));
}

fn el_set_attribute(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let name = arg_str(scope, &args, 0);
    let value = arg_str(scope, &args, 1);
    with_tree_mut(|t| t.set_attribute(h, &name, &value));
}

fn el_get_attribute(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let name = arg_str(scope, &args, 0);
    let val = with_tree(|t| t.get_attribute(h, &name).map(|s| s.to_string())).flatten();
    match val {
        Some(s) => rv.set(v8::String::new(scope, &s).unwrap().into()),
        None => rv.set(v8::null(scope).into()),
    }
}

fn el_has_attribute(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let name = arg_str(scope, &args, 0);
    let has = with_tree(|t| t.has_attribute(h, &name)).unwrap_or(false);
    rv.set(v8::Boolean::new(scope, has).into());
}

fn el_remove_attribute(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let name = arg_str(scope, &args, 0);
    with_tree_mut(|t| t.remove_attribute(h, &name));
}

/// element-scoped querySelector via NodeRef.
fn el_query_selector(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let sel = arg_str(scope, &args, 0);
    let found = with_tree(|t| NodeRef::new(t, h).query_selector(&sel).map(|n| n.handle())).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

// ---- innerHTML / outerHTML / children / form props / classList (P3.3 batch) -----------------

fn get_inner_html(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let html = with_tree(|t| turbo_dom_parser::rtdom::serialize::serialize_inner(t, h)).unwrap_or_default();
    rv.set(v8::String::new(scope, &html).unwrap().into());
}
fn set_inner_html(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, value: v8::Local<v8::Value>, args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let html = value.to_rust_string_lossy(scope);
    with_tree_mut(|t| t.set_inner_html(h, &html));
}
fn get_outer_html(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let html = with_tree(|t| turbo_dom_parser::rtdom::serialize::serialize_outer(t, h)).unwrap_or_default();
    rv.set(v8::String::new(scope, &html).unwrap().into());
}

/// `children` = element-only child nodes (HTMLCollection-ish array).
fn get_children(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let kids: Vec<Handle> = with_tree(|t| {
        t.children(h).into_iter().filter(|&c| t.node_type(c) == 1).collect()
    }).unwrap_or_default();
    let arr = v8::Array::new(scope, kids.len() as i32);
    for (i, k) in kids.into_iter().enumerate() {
        let node = wrap(scope, k);
        arr.set_index(scope, i as u32, node.into());
    }
    rv.set(arr.into());
}

fn get_parent_element(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let p = with_tree(|t| NodeRef::new(t, h).parent().map(|x| x.handle()).filter(|&ph| t.node_type(ph) == 1)).flatten();
    let v = wrap_opt(scope, p);
    rv.set(v);
}

fn get_first_element_child(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let c = with_tree(|t| NodeRef::new(t, h).first_element_child().map(|x| x.handle())).flatten();
    let v = wrap_opt(scope, c);
    rv.set(v);
}

fn get_namespace_uri(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, _args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    rv.set(v8::String::new(scope, "http://www.w3.org/1999/xhtml").unwrap().into());
}

/// `value`/`checked`/`disabled` etc — stored on the element as expandos by default (React's
/// controlled inputs set them directly); fall back to the attribute for `value`.
fn get_value(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let v = with_tree(|t| t.get_attribute(h, "value").map(|s| s.to_string())).flatten().unwrap_or_default();
    rv.set(v8::String::new(scope, &v).unwrap().into());
}
fn set_value(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, value: v8::Local<v8::Value>, args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let v = value.to_rust_string_lossy(scope);
    with_tree_mut(|t| t.set_attribute(h, "value", &v));
}

// no-op / stub element methods commonly called during render + queries.
fn el_focus(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_blur(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_click(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_scroll_into_view(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_get_bounding_client_rect(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let o = v8::Object::new(scope);
    for k in ["x", "y", "top", "left", "right", "bottom", "width", "height"] {
        let key = v8::String::new(scope, k).unwrap();
        let zero = v8::Number::new(scope, 0.0);
        o.set(scope, key.into(), zero.into());
    }
    rv.set(o.into());
}

/// `append(...nodes)` / `prepend(...)` / `remove()` (DOM ChildNode/ParentNode mixins).
fn el_append(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(parent) = handle_of(scope, args.this()) else { return };
    for i in 0..args.length() {
        if let Some(child) = args.get(i).to_object(scope).and_then(|o| handle_of(scope, o)) {
            with_tree_mut(|t| t.append_child(parent, child));
        }
    }
}
fn el_remove_self(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let parent = with_tree(|t| NodeRef::new(t, h).parent().map(|p| p.handle())).flatten();
    if let Some(p) = parent {
        with_tree_mut(|t| t.remove_child(p, h));
    }
}

// document native methods (reliable — no JS-bootstrap dependency).
fn doc_create_element_ns(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let tag = arg_str(scope, &args, 1);
    if let Some(h) = with_tree_mut(|t| t.create_element(&tag)) {
        let node = wrap(scope, h);
        rv.set(node.into());
    }
}
fn doc_create_fragment(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    if let Some(h) = with_tree_mut(|t| t.create_element("#document-fragment")) {
        let node = wrap(scope, h);
        rv.set(node.into());
    }
}
fn doc_create_comment(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let data = arg_str(scope, &args, 0);
    if let Some(h) = with_tree_mut(|t| t.create_comment(&data)) {
        let node = wrap(scope, h);
        rv.set(node.into());
    }
}

fn el_matches(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let sel = arg_str(scope, &args, 0);
    let m = with_tree(|t| t.matches(h, &sel)).unwrap_or(false);
    rv.set(v8::Boolean::new(scope, m).into());
}

fn el_contains(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let other = args.get(0).to_object(scope).and_then(|o| handle_of(scope, o));
    let contains = match other {
        Some(o) => with_tree(|t| {
            // walk up from `o` to see if `h` is an ancestor (or equal).
            let mut cur = Some(o);
            while let Some(c) = cur {
                if c == h { return true; }
                cur = NodeRef::new(t, c).parent().map(|p| p.handle());
            }
            false
        }).unwrap_or(false),
        None => false,
    };
    rv.set(v8::Boolean::new(scope, contains).into());
}

fn el_query_selector_all(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(h) = handle_of(scope, args.this()) else { return };
    let sel = arg_str(scope, &args, 0);
    let handles = with_tree(|t| {
        NodeRef::new(t, h).query_selector_all(&sel).into_iter().map(|n| n.handle()).collect::<Vec<_>>()
    })
    .unwrap_or_default();
    let arr = v8::Array::new(scope, handles.len() as i32);
    for (i, hh) in handles.into_iter().enumerate() {
        let node = wrap(scope, hh);
        arr.set_index(scope, i as u32, node.into());
    }
    rv.set(arr.into());
}

// ---- element accessors -----------------------------------------------------------------------
// Getter sig: (scope, name, PropertyCallbackArguments, ReturnValue<Value>).
// Setter sig: (scope, name, Local<Value>, PropertyCallbackArguments, ReturnValue<()>).

fn get_tag_name(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let tag = with_tree(|t| t.tag_name(h)).flatten().unwrap_or_default().to_uppercase();
    rv.set(v8::String::new(scope, &tag).unwrap().into());
}

fn get_text_content(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let txt = with_tree(|t| t.text_content(h)).unwrap_or_default();
    rv.set(v8::String::new(scope, &txt).unwrap().into());
}

fn set_text_content(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, value: v8::Local<v8::Value>, args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let txt = value.to_rust_string_lossy(scope);
    with_tree_mut(|t| t.set_text_content(h, &txt));
}

fn get_id(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let id = with_tree(|t| t.get_attribute(h, "id").map(|s| s.to_string())).flatten().unwrap_or_default();
    rv.set(v8::String::new(scope, &id).unwrap().into());
}

fn set_id(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, value: v8::Local<v8::Value>, args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let v = value.to_rust_string_lossy(scope);
    with_tree_mut(|t| t.set_attribute(h, "id", &v));
}

fn get_class_name(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let c = with_tree(|t| t.get_attribute(h, "class").map(|s| s.to_string())).flatten().unwrap_or_default();
    rv.set(v8::String::new(scope, &c).unwrap().into());
}

fn set_class_name(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, value: v8::Local<v8::Value>, args: v8::PropertyCallbackArguments, _rv: v8::ReturnValue<()>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let v = value.to_rust_string_lossy(scope);
    with_tree_mut(|t| t.set_attribute(h, "class", &v));
}

fn get_parent_node(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let found = with_tree(|t| NodeRef::new(t, h).parent().map(|x| x.handle())).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

fn get_first_child(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let found = with_tree(|t| NodeRef::new(t, h).first_child().map(|x| x.handle())).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

fn get_next_sibling(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let found = with_tree(|t| NodeRef::new(t, h).next_sibling().map(|x| x.handle())).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

// Event methods — no-op stubs for now (unblock render; real rtdom event dispatch is the next step).
fn el_add_event_listener(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_remove_event_listener(_scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {}
fn el_dispatch_event(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    rv.set(v8::Boolean::new(scope, true).into());
}

fn get_node_type(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let nt = with_tree(|t| t.node_type(h)).unwrap_or(1);
    rv.set(v8::Integer::new(scope, nt as i32).into());
}

fn get_node_name(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let (nt, tag) = with_tree(|t| (t.node_type(h), t.tag_name(h))).unwrap_or((1, None));
    let name = match nt {
        3 => "#text".to_string(),
        8 => "#comment".to_string(),
        9 => "#document".to_string(),
        11 => "#document-fragment".to_string(),
        _ => tag.unwrap_or_default().to_uppercase(),
    };
    rv.set(v8::String::new(scope, &name).unwrap().into());
}

fn get_child_nodes(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    let kids = with_tree(|t| t.children(h)).unwrap_or_default();
    let arr = v8::Array::new(scope, kids.len() as i32);
    for (i, k) in kids.into_iter().enumerate() {
        let node = wrap(scope, k);
        arr.set_index(scope, i as u32, node.into());
    }
    rv.set(arr.into());
}

fn get_owner_document(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, _args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    // every node's ownerDocument is the single document (root).
    if let Some(root) = with_tree(|t| t.root()) {
        let doc = wrap(scope, root);
        rv.set(doc.into());
    }
}

thread_local! {
    /// per-element `style` object (a plain JS object React reads/writes); cleared per file.
    static STYLE: RefCell<HashMap<Handle, v8::Global<v8::Object>>> = RefCell::new(HashMap::new());
}

/// `element.style` → a cached plain JS object (CSSStyleDeclaration-lite). React sets `style.color`
/// etc. as own props; `el.style.x === x` works. (Full CSSOM / attribute sync is a later refinement.)
fn get_style(scope: &mut v8::PinScope, _name: v8::Local<v8::Name>, args: v8::PropertyCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let Some(h) = handle_of(scope, args.holder()) else { return };
    if let Some(g) = STYLE.with(|s| s.borrow().get(&h).cloned()) {
        let o = v8::Local::new(scope, &g);
        rv.set(o.into());
        return;
    }
    let obj = v8::Object::new(scope);
    // setProperty/getPropertyValue/removeProperty so libs using the CSSOM methods don't crash.
    bind_method(scope, obj, "setProperty", style_set_property);
    bind_method(scope, obj, "getPropertyValue", style_get_property);
    bind_method(scope, obj, "removeProperty", style_remove_property);
    let g = v8::Global::new(scope, obj);
    STYLE.with(|s| { s.borrow_mut().insert(h, g); });
    rv.set(obj.into());
}

fn style_set_property(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let name = arg_str(scope, &args, 0);
    let value = args.get(1);
    if let Some(key) = v8::String::new(scope, &name) {
        args.this().set(scope, key.into(), value);
    }
}

fn style_get_property(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let name = arg_str(scope, &args, 0);
    if let Some(key) = v8::String::new(scope, &name) {
        if let Some(v) = args.this().get(scope, key.into()) {
            if !v.is_undefined() { rv.set(v); return; }
        }
    }
    rv.set(v8::String::new(scope, "").unwrap().into());
}

fn style_remove_property(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let name = arg_str(scope, &args, 0);
    if let Some(key) = v8::String::new(scope, &name) {
        args.this().delete(scope, key.into());
    }
}

// ---- document methods ------------------------------------------------------------------------

fn doc_create_element(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let tag = arg_str(scope, &args, 0);
    let Some(h) = with_tree_mut(|t| t.create_element(&tag)) else { return };
    let node = wrap(scope, h);
    rv.set(node.into());
}

fn doc_create_text_node(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let data = arg_str(scope, &args, 0);
    let Some(h) = with_tree_mut(|t| t.create_text_node(&data)) else { return };
    let node = wrap(scope, h);
    rv.set(node.into());
}

fn doc_get_element_by_id(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let id = arg_str(scope, &args, 0);
    let found = with_tree(|t| t.get_element_by_id(&id)).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

fn doc_query_selector(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let sel = arg_str(scope, &args, 0);
    let found = with_tree(|t| t.query_selector(&sel)).flatten();
    let v = wrap_opt(scope, found);
    rv.set(v);
}

fn doc_query_selector_all(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let sel = arg_str(scope, &args, 0);
    let handles: Vec<Handle> = with_tree(|t| t.query_selector_all(&sel).to_vec()).unwrap_or_default();
    let arr = v8::Array::new(scope, handles.len() as i32);
    for (i, hh) in handles.into_iter().enumerate() {
        let node = wrap(scope, hh);
        arr.set_index(scope, i as u32, node.into());
    }
    rv.set(arr.into());
}

// ---- install ---------------------------------------------------------------------------------

fn bind_method(scope: &mut v8::PinScope, obj: v8::Local<v8::Object>, name: &str, f: impl v8::MapFnTo<v8::FunctionCallback>) {
    let tmpl = v8::FunctionTemplate::new(scope, f);
    let func = tmpl.get_function(scope).unwrap();
    let key = v8::String::new(scope, name).unwrap();
    obj.set(scope, key.into(), func.into());
}

/// Set a method (FunctionTemplate) on an object template.
fn tmpl_method(scope: &mut v8::PinScope, tmpl: v8::Local<v8::ObjectTemplate>, name: &str, f: impl v8::MapFnTo<v8::FunctionCallback>) {
    let ft = v8::FunctionTemplate::new(scope, f);
    let key = v8::String::new(scope, name).unwrap();
    tmpl.set(key.into(), ft.into());
}

fn tmpl_getter(scope: &mut v8::PinScope, tmpl: v8::Local<v8::ObjectTemplate>, name: &str, g: impl v8::MapFnTo<v8::AccessorNameGetterCallback>) {
    let key = v8::String::new(scope, name).unwrap();
    tmpl.set_accessor(key.into(), g);
}

fn tmpl_accessor(scope: &mut v8::PinScope, tmpl: v8::Local<v8::ObjectTemplate>, name: &str, g: impl v8::MapFnTo<v8::AccessorNameGetterCallback>, s: impl v8::MapFnTo<v8::AccessorNameSetterCallback>) {
    let key = v8::String::new(scope, name).unwrap();
    tmpl.set_accessor_with_setter(key.into(), g, s);
}

/// Build the shared element template (methods + accessors).
fn build_el_template<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::ObjectTemplate> {
    let tmpl = v8::ObjectTemplate::new(scope);
    tmpl.set_internal_field_count(1);

    tmpl_method(scope, tmpl, "appendChild", el_append_child);
    tmpl_method(scope, tmpl, "removeChild", el_remove_child);
    tmpl_method(scope, tmpl, "insertBefore", el_insert_before);
    tmpl_method(scope, tmpl, "setAttribute", el_set_attribute);
    tmpl_method(scope, tmpl, "getAttribute", el_get_attribute);
    tmpl_method(scope, tmpl, "hasAttribute", el_has_attribute);
    tmpl_method(scope, tmpl, "removeAttribute", el_remove_attribute);
    tmpl_method(scope, tmpl, "querySelector", el_query_selector);
    tmpl_method(scope, tmpl, "querySelectorAll", el_query_selector_all);
    tmpl_method(scope, tmpl, "addEventListener", el_add_event_listener);
    tmpl_method(scope, tmpl, "removeEventListener", el_remove_event_listener);
    tmpl_method(scope, tmpl, "dispatchEvent", el_dispatch_event);
    tmpl_method(scope, tmpl, "matches", el_matches);
    tmpl_method(scope, tmpl, "contains", el_contains);
    tmpl_method(scope, tmpl, "append", el_append);
    tmpl_method(scope, tmpl, "prepend", el_append);
    tmpl_method(scope, tmpl, "remove", el_remove_self);
    tmpl_method(scope, tmpl, "focus", el_focus);
    tmpl_method(scope, tmpl, "blur", el_blur);
    tmpl_method(scope, tmpl, "click", el_click);
    tmpl_method(scope, tmpl, "scrollIntoView", el_scroll_into_view);
    tmpl_method(scope, tmpl, "getBoundingClientRect", el_get_bounding_client_rect);

    tmpl_getter(scope, tmpl, "tagName", get_tag_name);
    tmpl_getter(scope, tmpl, "parentNode", get_parent_node);
    tmpl_getter(scope, tmpl, "firstChild", get_first_child);
    tmpl_getter(scope, tmpl, "nextSibling", get_next_sibling);
    tmpl_getter(scope, tmpl, "style", get_style);
    tmpl_getter(scope, tmpl, "nodeType", get_node_type);
    tmpl_getter(scope, tmpl, "nodeName", get_node_name);
    tmpl_getter(scope, tmpl, "childNodes", get_child_nodes);
    tmpl_getter(scope, tmpl, "ownerDocument", get_owner_document);
    tmpl_getter(scope, tmpl, "outerHTML", get_outer_html);
    tmpl_getter(scope, tmpl, "children", get_children);
    tmpl_getter(scope, tmpl, "parentElement", get_parent_element);
    tmpl_getter(scope, tmpl, "firstElementChild", get_first_element_child);
    tmpl_getter(scope, tmpl, "namespaceURI", get_namespace_uri);
    tmpl_accessor(scope, tmpl, "innerHTML", get_inner_html, set_inner_html);
    tmpl_accessor(scope, tmpl, "value", get_value, set_value);

    tmpl_accessor(scope, tmpl, "textContent", get_text_content, set_text_content);
    tmpl_accessor(scope, tmpl, "id", get_id, set_id);
    tmpl_accessor(scope, tmpl, "className", get_class_name, set_class_name);

    // Always-on graceful-degradation interceptor: unimplemented members read as undefined (no
    // crash); recording the access list is gated by log_enabled() (TURBO_RUST_DOM_LOG).
    let handler = v8::NamedPropertyHandlerConfiguration::new()
        .getter(missing_getter)
        .setter(passthrough_setter)
        .flags(v8::PropertyHandlerFlags::NON_MASKING | v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS);
    tmpl.set_named_property_handler(handler);

    tmpl
}

/// Install `window`/`document` + the DOM onto `globalThis` for the current context.
pub fn install(scope: &mut v8::PinScope) {
    let tree = Tree::parse("<!DOCTYPE html><html><head></head><body></body></html>");
    let el_template = {
        let t = build_el_template(scope);
        v8::Global::new(scope, t)
    };
    DOM.with(|d| {
        *d.borrow_mut() = Some(DomState { tree, cache: HashMap::new(), el_template });
    });

    let root = with_tree(|t| t.root()).unwrap();
    let body_h = with_tree(|t| t.query_selector("body")).flatten();
    let html_h = with_tree(|t| t.query_selector("html")).flatten();

    // document = the root node object + document methods.
    let document = wrap(scope, root);
    bind_method(scope, document, "createElement", doc_create_element);
    bind_method(scope, document, "createTextNode", doc_create_text_node);
    bind_method(scope, document, "getElementById", doc_get_element_by_id);
    bind_method(scope, document, "querySelector", doc_query_selector);
    bind_method(scope, document, "querySelectorAll", doc_query_selector_all);
    bind_method(scope, document, "createElementNS", doc_create_element_ns);
    bind_method(scope, document, "createDocumentFragment", doc_create_fragment);
    bind_method(scope, document, "createComment", doc_create_comment);
    if let Some(b) = body_h {
        let body = wrap(scope, b);
        let key = v8::String::new(scope, "body").unwrap();
        document.set(scope, key.into(), body.into());
    }
    if let Some(html) = html_h {
        let de = wrap(scope, html);
        let key = v8::String::new(scope, "documentElement").unwrap();
        document.set(scope, key.into(), de.into());
    }

    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "document").unwrap();
    global.set(scope, key.into(), document.into());
    // window = globalThis
    let key = v8::String::new(scope, "window").unwrap();
    global.set(scope, key.into(), global.into());

    // Window-level globals that libs read as BARE identifiers (`navigator`, `HTMLElement`, …) — a
    // missing bare global is a ReferenceError the element interceptor can't catch, so they must
    // exist on globalThis. Plain-data / stub surface (refined natively as the logger surfaces hot
    // members); document extras that are pure JS go here too.
    run_js(scope, BOOTSTRAP);
}

/// Compile + run a JS snippet in the current context (best-effort).
fn run_js(scope: &mut v8::PinScope, src: &str) {
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    if let Some(code) = v8::String::new(tc, src) {
        if let Some(s) = v8::Script::compile(tc, code, None) {
            s.run(tc);
        }
    }
    if tc.has_caught() && log_enabled() {
        let msg = tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_default();
        eprintln!("[rust-dom] bootstrap threw: {msg}");
    }
}

const BOOTSTRAP: &str = r#"(function(){
  var g = globalThis;
  if (typeof g.navigator === 'undefined') g.navigator = { userAgent: 'turbo-test', platform: 'rust', language: 'en-US', languages: ['en-US'], clipboard: {}, maxTouchPoints: 0 };
  if (typeof g.getComputedStyle === 'undefined') g.getComputedStyle = function(){ return { getPropertyValue: function(){ return ''; } }; };
  if (typeof g.requestAnimationFrame === 'undefined') g.requestAnimationFrame = function(cb){ return setTimeout(function(){ cb(Date.now()); }, 0); };
  if (typeof g.cancelAnimationFrame === 'undefined') g.cancelAnimationFrame = function(id){ clearTimeout(id); };
  if (typeof g.matchMedia === 'undefined') g.matchMedia = function(q){ return { matches:false, media:q, addListener:function(){}, removeListener:function(){}, addEventListener:function(){}, removeEventListener:function(){}, dispatchEvent:function(){return false;} }; };
  if (typeof g.scrollTo === 'undefined') g.scrollTo = function(){};
  // DOM interface constructors (for `instanceof` / global presence). Stubs; identity not enforced.
  var ctors = ['Node','Element','HTMLElement','HTMLDivElement','HTMLInputElement','HTMLButtonElement','HTMLAnchorElement','HTMLSelectElement','HTMLTextAreaElement','HTMLFormElement','HTMLImageElement','HTMLLabelElement','HTMLOptionElement','HTMLUListElement','HTMLLIElement','HTMLSpanElement','HTMLParagraphElement','HTMLHeadingElement','HTMLTableElement','HTMLIFrameElement','HTMLCanvasElement','HTMLStyleElement','HTMLScriptElement','HTMLDocument','Document','DocumentFragment','ShadowRoot','Text','Comment','SVGElement','SVGSVGElement','DOMParser','EventTarget','AbortController','AbortSignal','DOMException',
    'UIEvent','MouseEvent','KeyboardEvent','FocusEvent','InputEvent','TouchEvent','PointerEvent','WheelEvent','DragEvent','ClipboardEvent','AnimationEvent','TransitionEvent','MessageEvent','ProgressEvent','CompositionEvent','PopStateEvent','HashChangeEvent','StorageEvent','ErrorEvent','CloseEvent'];
  ctors.forEach(function(n){ if (typeof g[n] === 'undefined') { var f = function(){}; f.prototype = {}; g[n] = f; } });
  // window-level event listeners (no-op until native events land).
  if (typeof g.addEventListener === 'undefined') g.addEventListener = function(){};
  if (typeof g.removeEventListener === 'undefined') g.removeEventListener = function(){};
  if (typeof g.dispatchEvent === 'undefined') g.dispatchEvent = function(){ return true; };
  // document extras (pure-JS shims over the native tree).
  var d = g.document;
  d.defaultView = g;
  d.documentMode = undefined;
  if (!d.head) { try { d.head = d.createElement('head'); if (d.documentElement) d.documentElement.appendChild(d.head); } catch(e){} }
  d.createElementNS = function(ns, tag){ return d.createElement(tag); };
  d.createDocumentFragment = function(){ return d.createElement('#document-fragment'); };
  d.createComment = function(data){ return d.createTextNode(data); };
  d.addEventListener = function(){};
  d.removeEventListener = function(){};
  d.dispatchEvent = function(){ return true; };
  d.activeElement = d.body;
})();"#;

/// Reset the per-thread DOM (call between test files when reusing an isolate).
pub fn reset() {
    STYLE.with(|s| s.borrow_mut().clear());
    if log_enabled() {
        dump_missing();
    }
    DOM.with(|d| *d.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();
    fn init_v8() {
        INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });
    }

    #[test]
    fn dom_slice_smoke() {
        init_v8();
        let isolate = &mut v8::Isolate::new(Default::default());
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        install(scope);

        let run = |scope: &mut v8::PinScope, code: &str| -> String {
            let src = v8::String::new(scope, code).unwrap();
            let script = v8::Script::compile(scope, src, None).unwrap();
            let r = script.run(scope).unwrap();
            r.to_rust_string_lossy(scope)
        };

        // create + append + attribute + textContent + querySelector + identity
        let out = run(scope, r#"
            const d = document.createElement('div');
            d.setAttribute('id', 'x');
            d.className = 'card';
            d.textContent = 'hello';
            document.body.appendChild(d);
            const found = document.querySelector('#x');
            JSON.stringify({
                tag: d.tagName,
                id: d.id,
                cls: d.className,
                text: found.textContent,
                identity: found === d,
                getAttr: d.getAttribute('id'),
                inBody: document.body.firstChild === d,
            });
        "#);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["tag"], "DIV", "{out}");
        assert_eq!(v["id"], "x", "{out}");
        assert_eq!(v["cls"], "card", "{out}");
        assert_eq!(v["text"], "hello", "{out}");
        assert_eq!(v["identity"], true, "querySelector must return the SAME object: {out}");
        assert_eq!(v["getAttr"], "x", "{out}");
        assert_eq!(v["inBody"], true, "{out}");
        reset();
    }
}
