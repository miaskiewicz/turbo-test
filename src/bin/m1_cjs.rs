//! M1.2 + M1.3 — CommonJS loader and ESM<->CJS interop (the M1 hard gate).
//!
//! This is "where homegrown runners die" (spec §M1). It proves three interop paths in
//! our own V8 embedding, no Node:
//!   - CJS requires CJS         (wrapper fn + native `require` + `module.exports`)
//!   - ESM imports CJS default  (default export == `module.exports`)
//!   - ESM imports CJS named    (named exports lifted off `module.exports`)
//!
//! Mechanism: each CJS file is evaluated eagerly via the classic CommonJS function
//! wrapper. For ESM consumers, the CJS module is also exposed as a V8 *synthetic
//! module* whose exports are filled from the evaluated `module.exports`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Esm,
    Cjs,
}

#[derive(Default)]
struct Registry {
    esm_by_path: HashMap<PathBuf, v8::Global<v8::Module>>,
    cjs_synth_by_path: HashMap<PathBuf, v8::Global<v8::Module>>,
    cjs_exports: HashMap<PathBuf, v8::Global<v8::Value>>,
    cjs_export_names: HashMap<PathBuf, Vec<String>>,
    path_by_hash: HashMap<i32, PathBuf>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn kind_of(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()) {
        Some("cjs") => Kind::Cjs,
        _ => Kind::Esm, // .mjs/.js treated as ESM in this spike
    }
}

fn resolve_spec(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    if !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
        eprintln!("bare specifier not yet supported (M4): {spec}");
        return None;
    }
    let base = from_dir.join(spec);
    for c in [
        base.clone(),
        base.with_extension("cjs"),
        base.with_extension("mjs"),
        base.with_extension("js"),
        base.join("index.js"),
    ] {
        if c.is_file() {
            return std::fs::canonicalize(c).ok();
        }
    }
    eprintln!("could not resolve {spec} from {}", from_dir.display());
    None
}

fn module_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(scope, name.into(), 0, 0, false, 123, None, false, false, true, None)
}

// ---- CommonJS -------------------------------------------------------------

/// Eagerly evaluate a CJS file via the CommonJS function wrapper, capture its
/// `module.exports`, then expose it to ESM as a synthetic module. Idempotent.
fn load_cjs<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path) -> Option<()> {
    if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(abs)) {
        return Some(());
    }

    let raw = std::fs::read_to_string(abs).ok()?;
    // Classic wrapper: source compiled to a function, then called with the CJS locals.
    let wrapped = format!(
        "(function (exports, module, require, __filename, __dirname) {{\n{raw}\n}})"
    );
    let code = v8::String::new(scope, &wrapped)?;
    let script = v8::Script::compile(scope, code, None)?;
    let wrapper_val = script.run(scope)?;
    let wrapper = v8::Local::<v8::Function>::try_from(wrapper_val).ok()?;

    // module = { exports: {} }
    let exports_obj = v8::Object::new(scope);
    let module_obj = v8::Object::new(scope);
    let exports_key = v8::String::new(scope, "exports")?;
    module_obj.set(scope, exports_key.into(), exports_obj.into());

    // require = __mkRequire(__dirname)
    let dir = abs.parent().unwrap();
    let dir_str = v8::String::new(scope, &dir.to_string_lossy())?;
    let global = scope.get_current_context().global(scope);
    let mk_key = v8::String::new(scope, "__mkRequire")?;
    let mk = v8::Local::<v8::Function>::try_from(global.get(scope, mk_key.into())?).ok()?;
    let undef = v8::undefined(scope).into();
    let require = mk.call(scope, undef, &[dir_str.into()])?;

    let filename = v8::String::new(scope, &abs.to_string_lossy())?;
    // call wrapper(exports, module, require, __filename, __dirname)
    wrapper.call(
        scope,
        undef,
        &[exports_obj.into(), module_obj.into(), require, filename.into(), dir_str.into()],
    )?;

    // module.exports may have been reassigned — read it back.
    let final_exports = module_obj.get(scope, exports_key.into())?;

    // detect named exports via Object.keys (helper installed in main)
    let names = object_keys(scope, global, final_exports);

    let exports_global = v8::Global::new(scope, final_exports);
    let synth = make_cjs_synthetic(scope, abs, &names)?;
    let synth_hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);

    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.cjs_exports.insert(abs.to_path_buf(), exports_global);
        reg.cjs_export_names.insert(abs.to_path_buf(), names);
        reg.cjs_synth_by_path.insert(abs.to_path_buf(), synth_global);
        reg.path_by_hash.insert(synth_hash, abs.to_path_buf());
    });
    Some(())
}

/// Call globalThis.__keys(obj) -> Vec<String> of own enumerable keys.
fn object_keys(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    obj: v8::Local<v8::Value>,
) -> Vec<String> {
    let keys_key = v8::String::new(scope, "__keys").unwrap();
    let keys_fn = match global
        .get(scope, keys_key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
    {
        Some(f) => f,
        None => return vec![],
    };
    let undef = v8::undefined(scope).into();
    let Some(res) = keys_fn.call(scope, undef, &[obj]) else {
        return vec![];
    };
    let Ok(arr) = v8::Local::<v8::Array>::try_from(res) else {
        return vec![];
    };
    let mut out = Vec::new();
    for i in 0..arr.length() {
        if let Some(v) = arr.get_index(scope, i) {
            out.push(v.to_rust_string_lossy(scope));
        }
    }
    out
}

/// Build a synthetic module exposing 'default' (= module.exports) + each named export.
fn make_cjs_synthetic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _abs: &Path,
    names: &[String],
) -> Option<v8::Local<'s, v8::Module>> {
    let mut export_names: Vec<v8::Local<v8::String>> = Vec::with_capacity(names.len() + 1);
    export_names.push(v8::String::new(scope, "default")?);
    for n in names {
        if n != "default" {
            export_names.push(v8::String::new(scope, n)?);
        }
    }
    let mod_name = v8::String::new(scope, "<cjs>")?;
    Some(v8::Module::create_synthetic_module(
        scope,
        mod_name,
        &export_names,
        cjs_synth_eval,
    ))
}

/// Synthetic eval steps: fill exports from the captured `module.exports`.
fn cjs_synth_eval<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let hash = module.get_identity_hash().get();

    let (exports_g, names) = REGISTRY.with(|r| {
        let reg = r.borrow();
        let path = reg.path_by_hash.get(&hash)?.clone();
        let g = reg.cjs_exports.get(&path)?.clone();
        let n = reg.cjs_export_names.get(&path).cloned().unwrap_or_default();
        Some((g, n))
    })?;

    let exports = v8::Local::new(scope, &exports_g);

    // default = module.exports
    let dkey = v8::String::new(scope, "default")?;
    module.set_synthetic_module_export(scope, dkey, exports)?;

    // named = module.exports[name]
    if let Ok(obj) = v8::Local::<v8::Object>::try_from(exports) {
        for n in names {
            if n == "default" {
                continue;
            }
            let k = v8::String::new(scope, &n)?;
            if let Some(v) = obj.get(scope, k.into()) {
                module.set_synthetic_module_export(scope, k, v)?;
            }
        }
    }
    Some(v8::undefined(scope).into())
}

/// Native require(dir, spec) — used by the CJS wrapper for CJS->CJS requires.
fn native_require(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    let Some(abs) = resolve_spec(&spec, Path::new(&dir)) else {
        throw(scope, &format!("cannot resolve {spec}"));
        return;
    };
    if kind_of(&abs) != Kind::Cjs {
        throw(scope, &format!("require() of ESM not supported (M1): {spec}"));
        return;
    }
    if load_cjs(scope, &abs).is_none() {
        throw(scope, &format!("failed to load {spec}"));
        return;
    }
    let exports = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned());
    if let Some(g) = exports {
        let local = v8::Local::new(scope, &g);
        rv.set(local);
    }
}

fn throw(scope: &mut v8::PinScope, msg: &str) {
    let m = v8::String::new(scope, msg).unwrap();
    let exc = v8::Exception::error(scope, m);
    scope.throw_exception(exc);
}

// ---- ESM graph (with CJS deps) -------------------------------------------

fn load_graph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    if let Some(g) = REGISTRY.with(|r| r.borrow().esm_by_path.get(abs).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }

    let raw = std::fs::read_to_string(abs).ok()?;
    let code = v8::String::new(scope, &raw)?;
    let name = v8::String::new(scope, &abs.to_string_lossy())?;
    let origin = module_origin(scope, name);
    let mut source = v8::script_compiler::Source::new(code, Some(&origin));
    let module = v8::script_compiler::compile_module(scope, &mut source)?;

    let hash = module.get_identity_hash().get();
    let global = v8::Global::new(scope, module);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.esm_by_path.insert(abs.to_path_buf(), global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
    });

    let requests = module.get_module_requests();
    let dir = abs.parent().unwrap().to_path_buf();
    for i in 0..requests.length() {
        let req = v8::Local::<v8::ModuleRequest>::try_from(requests.get(scope, i).unwrap()).unwrap();
        let spec = req.get_specifier().to_rust_string_lossy(scope);
        let dep = resolve_spec(&spec, &dir)?;
        match kind_of(&dep) {
            Kind::Esm => {
                load_graph(scope, &dep)?;
            }
            Kind::Cjs => {
                load_cjs(scope, &dep)?;
            }
        }
    }
    Some(module)
}

/// Resolve callback: returns ESM module or CJS synthetic module from the registry.
fn resolve_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _attrs: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let spec = specifier.to_rust_string_lossy(scope);
    let ref_hash = referrer.get_identity_hash().get();
    REGISTRY.with(|r| {
        let reg = r.borrow();
        let from = reg.path_by_hash.get(&ref_hash)?;
        let abs = resolve_spec(&spec, from.parent()?)?;
        if let Some(g) = reg.esm_by_path.get(&abs) {
            return Some(v8::Local::new(scope, g));
        }
        if let Some(g) = reg.cjs_synth_by_path.get(&abs) {
            return Some(v8::Local::new(scope, g));
        }
        None
    })
}

// ---- bootstrap ------------------------------------------------------------

fn install_helpers(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    // log
    let f = |scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue| {
        let mut parts = Vec::new();
        for i in 0..args.length() {
            parts.push(args.get(i).to_rust_string_lossy(scope));
        }
        println!("[js] {}", parts.join(" "));
    };
    bind_fn(scope, global, "log", f);
    bind_fn(scope, global, "__nativeRequire", native_require);

    // __keys + __mkRequire as JS
    let setup = v8::String::new(
        scope,
        "globalThis.__keys = (o) => (o && (typeof o === 'object' || typeof o === 'function')) ? Object.keys(o) : [];\
         globalThis.__mkRequire = (dir) => (spec) => globalThis.__nativeRequire(dir, spec);",
    )
    .unwrap();
    let s = v8::Script::compile(scope, setup, None).unwrap();
    s.run(scope).unwrap();
}

fn bind_fn(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    name: &str,
    f: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let tmpl = v8::FunctionTemplate::new(scope, f);
    let func = tmpl.get_function(scope).unwrap();
    let key = v8::String::new(scope, name).unwrap();
    global.set(scope, key.into(), func.into());
}

fn main() {
    let platform = v8::new_default_platform(0, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();

    let entry = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/interop/entry.mjs".to_string());
    let entry_abs = std::fs::canonicalize(&entry).expect("entry not found");
    println!("turbo-test M1.2/M1.3 — CJS + ESM<->CJS interop");
    println!("entry: {}\n", entry_abs.display());

    let exit_code;
    {
        let isolate = &mut v8::Isolate::new(v8::Isolate::create_params());
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        install_helpers(scope, global);

        let module = match load_graph(scope, &entry_abs) {
            Some(m) => m,
            None => {
                eprintln!("graph load failed");
                clear_registry();
                std::process::exit(1);
            }
        };
        println!(
            "loaded: {} esm, {} cjs",
            REGISTRY.with(|r| r.borrow().esm_by_path.len()),
            REGISTRY.with(|r| r.borrow().cjs_synth_by_path.len())
        );

        if module.instantiate_module(scope, resolve_callback) != Some(true) {
            eprintln!("instantiate failed");
            clear_registry();
            std::process::exit(1);
        }
        let result = module.evaluate(scope);
        scope.perform_microtask_checkpoint();
        if result.is_none() || module.get_status() != v8::ModuleStatus::Evaluated {
            eprintln!("evaluate failed: status={:?}", module.get_status());
            clear_registry();
            std::process::exit(1);
        }

        let key = v8::String::new(scope, "__out").unwrap();
        let got = global.get(scope, key.into()).unwrap().to_rust_string_lossy(scope);
        let expected = "hi esm|1.0.0|hi cjs v1.0.0";
        println!("\nglobalThis.__out = {got:?}");
        println!("expected         = {expected:?}");
        let pass = got == expected;
        println!("{}", if pass { "==> M1.2/M1.3 INTEROP PASS" } else { "==> FAIL" });
        exit_code = if pass { 0 } else { 1 };
        clear_registry();
    }
    std::process::exit(exit_code);
}

fn clear_registry() {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.esm_by_path.clear();
        reg.cjs_synth_by_path.clear();
        reg.cjs_exports.clear();
        reg.cjs_export_names.clear();
        reg.path_by_hash.clear();
    });
}
