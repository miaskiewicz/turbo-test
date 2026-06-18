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
    let handles = with_tree(|t| t.query_selector_all(&sel)).unwrap_or_default();
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

    tmpl_getter(scope, tmpl, "tagName", get_tag_name);
    tmpl_getter(scope, tmpl, "parentNode", get_parent_node);
    tmpl_getter(scope, tmpl, "firstChild", get_first_child);
    tmpl_getter(scope, tmpl, "nextSibling", get_next_sibling);

    tmpl_accessor(scope, tmpl, "textContent", get_text_content, set_text_content);
    tmpl_accessor(scope, tmpl, "id", get_id, set_id);
    tmpl_accessor(scope, tmpl, "className", get_class_name, set_class_name);

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
}

/// Reset the per-thread DOM (call between test files when reusing an isolate).
pub fn reset() {
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
