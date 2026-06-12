//! M1.1 — native ESM module loader (the spine of the module runner).
//!
//! Proves the V8 mechanics the whole runner hangs on: compile a real multi-file ESM
//! graph from disk into `v8::Module`s, resolve cross-module imports through a host
//! resolve callback, instantiate, evaluate, and pump microtasks — all in our own V8
//! embedding (no Node). Exercises a shared dependency imported via two paths (must be
//! the SAME module instance) which is the property that makes live bindings work.
//!
//! Layers stacked on this later in M1: CJS + interop, oxc transform for TS, vi.mock
//! interception at the registry level.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-thread module registry. The V8 resolve callback is a non-capturing `fn`, so it
/// cannot close over state — it reaches the graph through this thread-local instead.
/// Keyed both ways: absolute path -> compiled module, and module identity-hash ->
/// path (so the callback can find the referrer's directory for relative resolution).
#[derive(Default)]
struct Registry {
    by_path: HashMap<PathBuf, v8::Global<v8::Module>>,
    path_by_hash: HashMap<i32, PathBuf>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Transform hook — now wired to oxc: `.ts/.tsx/.jsx/.mts/.cts` are transformed to JS
/// before V8 compiles them; plain JS/MJS passes through.
fn transform(path: &Path, src: String) -> String {
    turbo_test::transform::maybe_transform(path, src).unwrap_or_else(|e| {
        eprintln!("transform {}: {e}", path.display());
        String::new()
    })
}

/// Resolve an import specifier relative to the importing file's directory.
/// M1.1 handles relative specifiers with extension probing; bare specifiers
/// (node_modules) come with oxc_resolver in M4.
fn resolve_spec(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    if !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
        eprintln!("bare specifier not yet supported (M4): {spec}");
        return None;
    }
    let base = from_dir.join(spec);
    let candidates = [
        base.clone(),
        base.with_extension("mjs"),
        base.with_extension("js"),
        base.with_extension("mts"),
        base.with_extension("ts"),
        base.with_extension("tsx"),
        base.join("index.mjs"),
        base.join("index.js"),
        base.join("index.ts"),
    ];
    for c in candidates {
        if c.is_file() {
            return std::fs::canonicalize(c).ok();
        }
    }
    eprintln!("could not resolve {spec} from {}", from_dir.display());
    None
}

/// Recursively compile + register the whole import graph reachable from `abs_path`.
/// Registration happens BEFORE recursing into deps so import cycles terminate.
fn load_graph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs_path: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    // already loaded? return the cached module
    if let Some(g) = REGISTRY.with(|r| r.borrow().by_path.get(abs_path).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }

    let raw = std::fs::read_to_string(abs_path)
        .map_err(|e| eprintln!("read {}: {e}", abs_path.display()))
        .ok()?;
    let src = transform(abs_path, raw);

    let code = v8::String::new(scope, &src)?;
    let name = v8::String::new(scope, &abs_path.to_string_lossy())?;
    let origin = module_origin(scope, name);
    let mut source = v8::script_compiler::Source::new(code, Some(&origin));
    let module = v8::script_compiler::compile_module(scope, &mut source)?;

    // register before recursing (cycle-safe)
    let hash = module.get_identity_hash().get();
    let global = v8::Global::new(scope, module);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.by_path.insert(abs_path.to_path_buf(), global);
        reg.path_by_hash.insert(hash, abs_path.to_path_buf());
    });

    // discover + load dependencies
    let requests = module.get_module_requests();
    let dir = abs_path.parent().unwrap().to_path_buf();
    for i in 0..requests.length() {
        let req = v8::Local::<v8::ModuleRequest>::try_from(requests.get(scope, i).unwrap()).unwrap();
        let spec = req.get_specifier().to_rust_string_lossy(scope);
        if let Some(dep) = resolve_spec(&spec, &dir) {
            if load_graph(scope, &dep).is_none() {
                return None;
            }
        } else {
            return None;
        }
    }

    Some(module)
}

/// Host resolve callback: V8 calls this during instantiate for each import. Maps
/// (referrer, specifier) back to an already-compiled module from the registry.
fn resolve_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let spec = specifier.to_rust_string_lossy(scope);
    let ref_hash = referrer.get_identity_hash().get();

    REGISTRY.with(|r| {
        let reg = r.borrow();
        let from = reg.path_by_hash.get(&ref_hash)?;
        let abs = resolve_spec(&spec, from.parent()?)?;
        let g = reg.by_path.get(&abs)?;
        Some(v8::Local::new(scope, g))
    })
}

fn module_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: v8::Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(
        scope,
        resource_name.into(),
        0,
        0,
        false,
        123,
        None,
        false,
        false,
        true, // is_module
        None, // host_defined_options
    )
}

/// Minimal `log(...)` binding so fixtures can emit, proving real execution.
fn install_log(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    let key = v8::String::new(scope, "log").unwrap();
    let tmpl = v8::FunctionTemplate::new(scope, |scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue| {
        let mut parts = Vec::new();
        for i in 0..args.length() {
            parts.push(args.get(i).to_rust_string_lossy(scope));
        }
        println!("[js] {}", parts.join(" "));
    });
    let func = tmpl.get_function(scope).unwrap();
    global.set(scope, key.into(), func.into());
}

fn main() {
    let platform = v8::new_default_platform(0, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();

    let entry = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/esm/entry.mjs".to_string());
    let entry_abs = std::fs::canonicalize(&entry).expect("entry file not found");
    println!("turbo-test M1.1 — native ESM loader");
    println!("entry: {}\n", entry_abs.display());

    let exit_code;
    {
        let isolate = &mut v8::Isolate::new(v8::Isolate::create_params());
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let global = context.global(scope);
        install_log(scope, global);

        // 1. compile + register the whole graph
        let module = match load_graph(scope, &entry_abs) {
            Some(m) => m,
            None => {
                eprintln!("graph load failed");
                REGISTRY.with(|r| r.borrow_mut().by_path.clear());
                std::process::exit(1);
            }
        };
        let graph_size = REGISTRY.with(|r| r.borrow().by_path.len());
        println!("graph compiled: {graph_size} modules");

        // 2. instantiate (drives the resolve callback)
        let ok = module.instantiate_module(scope, resolve_callback);
        if ok != Some(true) {
            eprintln!("instantiate failed");
            std::process::exit(1);
        }
        println!("instantiated: status={:?}", module.get_status());

        // 3. evaluate + settle microtasks
        let result = module.evaluate(scope);
        scope.perform_microtask_checkpoint();
        if result.is_none() || module.get_status() != v8::ModuleStatus::Evaluated {
            eprintln!("evaluate failed: status={:?}", module.get_status());
            std::process::exit(1);
        }

        // module evaluation returns a promise; confirm it fulfilled
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result.unwrap()) {
            if promise.state() == v8::PromiseState::Rejected {
                eprintln!("module promise rejected");
                std::process::exit(1);
            }
        }
        println!("evaluated: status={:?}", module.get_status());

        // 4. read back globalThis.__result to prove cross-module execution
        let key = v8::String::new(scope, "__result").unwrap();
        let val = global.get(scope, key.into()).unwrap();
        let got = val.to_rust_string_lossy(scope);
        let expected = "9";
        println!("\nglobalThis.__result = {got}  (expected {expected})");
        let pass = got == expected;
        println!("{}", if pass { "==> M1.1 ESM PASS" } else { "==> M1.1 ESM FAIL" });
        exit_code = if pass { 0 } else { 1 };

        // clear registry while isolate still alive (Global drop needs it)
        REGISTRY.with(|r| {
            let mut reg = r.borrow_mut();
            reg.by_path.clear();
            reg.path_by_hash.clear();
        });
    }
    std::process::exit(exit_code);
}
