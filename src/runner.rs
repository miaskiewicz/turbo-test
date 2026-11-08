//! M1 consolidated ModuleRunner — the end-to-end native module substrate.
//!
//! Folds the proven spikes (ESM loader, CJS + interop, oxc transform) into one engine
//! and adds the remaining M1 surface: `import.meta`, dynamic `import()`, and `vi.mock`
//! interception/hoisting at the loader level. Ships a minimal test runtime
//! (describe/it/expect/vi) so real logic test files run end-to-end and report pass/fail.
//! The minimal runtime is a placeholder for the REAL @vitest/* framework baked in at M3.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Esm,
    Cjs,
}

/// A graph request's view of a CommonJS module (`esm_cjs::graph_interop_rewrite`).
#[derive(Clone, Copy, PartialEq)]
enum Variant {
    /// `?__tt_ns`: its default export is vitest's interop namespace object
    Ns,
    /// `?__tt_star`: that namespace's names (what `export *` re-exports)
    Star,
    /// `?__tt_node`: Node's namespace (default `module.exports`, `'module.exports'`, names)
    Node,
}

#[derive(Default)]
struct Registry {
    esm_by_path: HashMap<PathBuf, v8::Global<v8::Module>>,
    cjs_synth_by_path: HashMap<PathBuf, v8::Global<v8::Module>>,
    cjs_exports: HashMap<PathBuf, v8::Global<v8::Value>>,
    /// Genuine (unmocked) exports of every real module loaded via load_cjs, kept even after a
    /// mock overwrites cjs_exports — so vi.importActual/importOriginal returns the real module
    /// without re-running it (avoids dual instances of singletons like react-query).
    real_exports: HashMap<PathBuf, v8::Global<v8::Value>>,
    /// The REAL ES module record of a mocked path, linked by an external package's own `import`
    /// (Node loads that package, never seeing vi.mock; turbo-test#218).
    real_esm: HashMap<PathBuf, v8::Global<v8::Module>>,
    cjs_export_names: HashMap<PathBuf, Vec<String>>,
    path_by_hash: HashMap<i32, PathBuf>,
    /// The ES-module graph's synthetic modules of a CommonJS module's namespace (`?__tt_ns` /
    /// `?__tt_star` / `?__tt_node` requests, turbo-test#133): module hash -> (module path, kind,
    /// export names).
    variant_by_hash: HashMap<i32, (PathBuf, Variant, Vec<String>)>,
    /// vi.mock interception: resolved path -> factory source. Consulted on load.
    mocks: HashMap<PathBuf, String>,
    /// CJS modules currently executing (path -> their `module` object). A circular require()
    /// returns the in-progress module's live `exports` (Node behavior) instead of reloading.
    loading: HashMap<PathBuf, v8::Global<v8::Object>>,
    /// Named imports requested from a mock path by bundle code — so the synthetic module
    /// exposes them even if the mock factory omitted them (vitest's lenient behavior:
    /// missing named exports read as undefined rather than a link-time SyntaxError).
    extra_exports: HashMap<PathBuf, Vec<String>>,
    /// Layer-B lazy module stubs (TURBO_LAZY_STUBS): per-isolate Proxy namespace per barrel
    /// specifier. Built once on first require, reused for every importer in this file's isolate.
    lazy_stub_ns: HashMap<String, v8::Global<v8::Value>>,
    /// Reverse dependency map for isolate-reuse mock-graph invalidation: imported path -> set of
    /// paths that require()d it. When a file mocks module M, every (transitive) importer of M is
    /// evicted so it re-imports the mock instead of the version it captured under an old mock.
    import_edges: HashMap<PathBuf, std::collections::HashSet<PathBuf>>,
    /// Generated ESM bundle (cache path) -> the ORIGINAL source dir it was built from. The bundle
    /// lives in the cache dir (no node_modules beside it), so resolve_callback resolves its
    /// externalized bare imports relative to this dir instead of the cache file's location.
    bundle_src_dir: HashMap<PathBuf, PathBuf>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
    /// Force a single file onto the FRESH (per-file isolate) path even when reuse is enabled —
    /// used by the fresh-retry: a file that failed under reuse is re-run in a clean isolate
    /// (authoritative) so cross-file leak artifacts don't count as real failures.
    static FORCE_FRESH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Isolate-reuse (TURBO_REUSE_ISOLATE): one persistent isolate + context per worker thread,
    /// reused across files so node_modules barrels are evaluated once, not per file.
    static REUSE_ISO: RefCell<Option<v8::OwnedIsolate>> = const { RefCell::new(None) };
    static REUSE_CTX: RefCell<Option<v8::Global<v8::Context>>> = const { RefCell::new(None) };
    static DOM_INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// CommonJS-first module resolution (jest/node backend projects). Set once per run from the
    /// entry's project: a jest config with a node test environment resolves the `require` export
    /// condition (the build Node uses), so sequelize/tslib/lexical/etc. get their working CJS
    /// build instead of an ESM build that breaks once bundled to CJS (imports too). Off (ESM-first:
    /// Node's conditions, `import` for an import and `require` for a require(), turbo-test#134)
    /// for vitest/React projects.
    static CJS_FIRST_RESOLUTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while resolve+drain of pending vi.mock factories is in progress. An async factory's
    /// `await import('<self>')` re-enters load_cjs(drain_mocks=true) for the real module; without
    /// this guard that nested call would drain (and clear) the very queue we're mid-resolving,
    /// registering the still-pending factory promise as an empty mock. While set, load_cjs leaves
    /// the pending-mock queue untouched so the outer drain registers the resolved exports.
    static RESOLVING_MOCKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Mock targets registered by setup files (run once per worker) — kept across files in the
    /// reuse path so e.g. the analytics mock survives, while per-file test mocks are evicted.
    static SETUP_MOCKS: RefCell<std::collections::HashSet<PathBuf>> = RefCell::new(std::collections::HashSet::new());
    /// The error of a setup file that threw when the reused isolate ran them (once per worker):
    /// every file on that isolate fails with it, as each would under vitest.
    static REUSE_SETUP_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Snapshot (shallow clone) of each setup-mock's exports taken after the first file. A test
    /// file that re-mocks a setup path mutates the shared exports in place (to preserve identity
    /// for cached importers); restoring from this snapshot before the next file undoes that, so
    /// one file's analytics mock doesn't leak into the rest.
    static SETUP_MOCK_SNAPSHOT: RefCell<HashMap<PathBuf, v8::Global<v8::Value>>> = RefCell::new(HashMap::new());
}

/// The registered CommonJS exports of the items' paths that have them: (path, item, exports).
fn live_cjs_exports<T>(items: Vec<(PathBuf, T)>) -> Vec<(PathBuf, T, v8::Global<v8::Value>)> {
    REGISTRY.with(|r| {
        let reg = r.borrow();
        let live = |(p, t): (PathBuf, T)| Some((reg.cjs_exports.get(&p)?.clone(), p, t));
        items
            .into_iter()
            .filter_map(live)
            .map(|(g, p, t)| (p, t, g))
            .collect()
    })
}

/// Shallow-clone each setup-mock's current exports into SETUP_MOCK_SNAPSHOT (called once, after
/// the first file's setup runs).
fn snapshot_setup_mocks(scope: &mut v8::PinScope) {
    let paths: Vec<(PathBuf, ())> =
        SETUP_MOCKS.with(|s| s.borrow().iter().map(|p| (p.clone(), ())).collect());
    let global = scope.get_current_context().global(scope);
    for (p, (), g) in live_cjs_exports(paths) {
        let obj = v8::Local::new(scope, &g);
        // (exports that are not an object have nothing to snapshot)
        if let Some(o) = obj.to_object(scope) {
            let names = object_keys(scope, global, obj);
            let clone = v8::Object::new(scope);
            for n in &names {
                if let Some(k) = v8::String::new(scope, n) {
                    if let Some(v) = o.get(scope, k.into()) {
                        clone.set(scope, k.into(), v);
                    }
                }
            }
            let clone_val: v8::Local<v8::Value> = clone.into();
            let cg = v8::Global::new(scope, clone_val);
            SETUP_MOCK_SNAPSHOT.with(|s| s.borrow_mut().insert(p, cg));
        }
    }
}

/// Restore each setup-mock's exports from its snapshot (called before every file after the
/// first), copying the saved props back onto the live (identity-preserved) exports object.
fn restore_setup_mocks(scope: &mut v8::PinScope) {
    let snaps: Vec<(PathBuf, v8::Global<v8::Value>)> = SETUP_MOCK_SNAPSHOT.with(|s| {
        s.borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    });
    let global = scope.get_current_context().global(scope);
    for (_, cg, lg) in live_cjs_exports(snaps) {
        let live = v8::Local::new(scope, &lg);
        let clone = v8::Local::new(scope, &cg);
        if let (Some(dst), Some(src)) = (live.to_object(scope), clone.to_object(scope)) {
            let names = object_keys(scope, global, clone);
            for n in &names {
                if let Some(k) = v8::String::new(scope, n) {
                    if let Some(v) = src.get(scope, k.into()) {
                        dst.set(scope, k.into(), v);
                    }
                }
            }
        }
    }
}

/// Zero-config speed mode: reuse the worker's isolate across files (like vitest `isolate:false`).
/// Reads the decision cached by reuse_decision() (set at the top of run_test_file); false until then.
fn reuse_isolate_enabled() -> bool {
    REUSE_DECISION.get().copied().unwrap_or(false)
}

/// Whether the CURRENT file runs on the reuse path (the persistent isolate). Under the reuse
/// switch some files still run on a fresh per-file isolate: the fresh-isolate retry
/// (FORCE_FRESH) and every file under `--coverage` (run_test_file's dispatch). Mock
/// registration's reuse-only behaviour (patching a re-mock into the existing exports object,
/// mock-graph invalidation) must follow the path, not the switch.
fn on_reuse_path() -> bool {
    reuse_isolate_enabled() && !FORCE_FRESH.with(|f| f.get()) && !crate::coverage::enabled()
}

/// Mock-graph invalidation: when `target` is (re-)mocked, evict every app module that
/// (transitively) require()d it, so each re-imports the new mock instead of the stale version it
/// captured under a prior mock/real load. node_modules importers are NOT evicted (re-evaluating
/// them would re-trigger the emotion/MUI singleton accumulation that keeping modules warm avoids).
/// A MOCKED importer is neither evicted nor walked through: its exports are its mock, which does
/// not depend on what it imports (evicting them left it "mocked but no exports", read as `{}`),
/// and its importers already see that mock.
fn invalidate_importers(target: &Path) {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let mut stack = vec![target.to_path_buf()];
        let mut seen = std::collections::HashSet::new();
        let mut to_evict: Vec<PathBuf> = Vec::new();
        while let Some(m) = stack.pop() {
            let importers = reg.import_edges.get(&m).cloned().unwrap_or_default();
            for imp in importers {
                if imp.to_string_lossy().contains("/node_modules/") || reg.mocks.contains_key(&imp)
                {
                    continue;
                }
                if !seen.insert(imp.clone()) {
                    continue;
                }
                to_evict.push(imp.clone());
                stack.push(imp);
            }
        }
        for p in &to_evict {
            reg.cjs_exports.remove(p);
            reg.cjs_synth_by_path.remove(p);
            reg.cjs_export_names.remove(p);
            reg.real_exports.remove(p);
            reg.esm_by_path.remove(p);
        }
    });
}

/// The ES modules of the registry that a failed file left linked but never evaluated: a module
/// that threw while the graph evaluated stops the modules after it, which stay bound to that
/// file's mocks (turbo-test#79). Evaluating one in a later file would evaluate those stale
/// links, so the reuse reset drops them and the next importer loads them afresh.
fn unevaluated_modules(scope: &mut v8::PinScope) -> Vec<PathBuf> {
    let modules: Vec<(PathBuf, v8::Global<v8::Module>)> = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.esm_by_path
            .iter()
            .map(|(p, m)| (p.clone(), m.clone()))
            .collect()
    });
    modules
        .into_iter()
        .filter(|(_, m)| {
            !matches!(
                v8::Local::new(scope, m).get_status(),
                v8::ModuleStatus::Evaluated | v8::ModuleStatus::Errored
            )
        })
        .map(|(p, _)| p)
        .collect()
}

/// Per-file registry reset for the reuse path: drop app modules + ALL mocks, but KEEP genuinely
/// evaluated node_modules modules so the next file reuses them (the whole point). Any node_modules
/// path that was MOCKED in the prior file is evicted too, so the next file gets the real module,
/// and so is every module in `stale` (unevaluated_modules).
fn reset_app_registry(stale: &[PathBuf]) {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let setup_mocks = SETUP_MOCKS.with(|s| s.borrow().clone());
        // vitest isolate:false caches the WHOLE module graph (app + node_modules) across files
        // and only re-runs each unique entry test file. Re-evaluating app modules per file would
        // re-register into kept singletons (emotion/MUI styled caches, react-transition-group)
        // until rendering breaks (~100 files in). So KEEP every evaluated module; evict only the
        // modules a TEST file mocked (not setup mocks), so the next file re-resolves the real
        // module (or applies its own mock). The entry test files are each loaded once (unique
        // path) so they never collide; their describe/it re-run because each is a fresh entry.
        let test_mocked: Vec<PathBuf> = reg
            .mocks
            .keys()
            .filter(|p| !setup_mocks.contains(*p))
            .cloned()
            .collect();
        let evict = |p: &PathBuf| test_mocked.contains(p) || stale.contains(p);
        reg.cjs_exports.retain(|p, _| !evict(p));
        reg.cjs_synth_by_path.retain(|p, _| !evict(p));
        reg.cjs_export_names.retain(|p, _| !evict(p));
        reg.real_exports.retain(|p, _| !evict(p));
        reg.real_esm.retain(|p, _| !evict(p));
        reg.esm_by_path.retain(|p, _| !evict(p));
        let live: std::collections::HashSet<PathBuf> = reg
            .cjs_exports
            .keys()
            .chain(reg.esm_by_path.keys())
            .cloned()
            .collect();
        reg.path_by_hash.retain(|_, p| live.contains(p));
        reg.variant_by_hash.retain(|_, (p, _, _)| live.contains(p));
        // keep setup mocks in the mock registry; drop test-file mocks
        reg.mocks.retain(|p, _| setup_mocks.contains(p));
        reg.extra_exports.clear();
        reg.loading.clear();
        reg.lazy_stub_ns.clear();
    });
}

/// Parsed `resolve.alias`/`test.alias` (from launcher's TURBO_ALIAS), as oxc alias entries. Cached
/// for the process — the config is fixed for a whole run. Empty when no aliases are configured.
fn config_alias_list() -> oxc_resolver::Alias {
    thread_local! {
        static ALIASES: RefCell<Option<oxc_resolver::Alias>> = const { RefCell::new(None) };
    }
    ALIASES.with(|c| {
        if let Some(a) = c.borrow().as_ref() {
            return a.clone();
        }
        let parsed: oxc_resolver::Alias = std::env::var("TURBO_ALIAS")
            .ok()
            .map(|raw| parse_alias_list(&raw))
            .unwrap_or_default();
        *c.borrow_mut() = Some(parsed.clone());
        parsed
    })
}

/// Parse the TURBO_ALIAS wire format: records separated by U+001E, each `key U+001F path`.
/// Records with an empty key or path (or no separator) are dropped.
fn parse_alias_list(raw: &str) -> oxc_resolver::Alias {
    raw.split('\u{1e}')
        .filter_map(|rec| {
            let (k, v) = rec.split_once('\u{1f}')?;
            if k.is_empty() || v.is_empty() {
                return None;
            }
            Some((
                k.to_string(),
                vec![oxc_resolver::AliasValue::Path(v.to_string())],
            ))
        })
        .collect()
}

/// Cached node-resolution Resolver (oxc_resolver) for bare specifiers / node_modules.
fn base_resolve_options(
    tsconfig: Option<PathBuf>,
    esm: bool,
    require_fallback: bool,
) -> oxc_resolver::ResolveOptions {
    let strs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // As Node resolves for vitest (which runs packages through Node, turbo-test#134): an `import`
    // matches a package's `import` condition, a `require()` its `require` condition; either reads
    // `main` before `module` (Node ignores `module`; it stays the fallback of a package without
    // `main`). A package without a dual build is then one instance for both; one with an
    // `import` and a `require` build is two, as under Node (the dual-package hazard is the
    // package's). `module` is not a condition (vitest 4.1.12 loads `{ module, default }`'s
    // `default` build, as Node); the `module` main field still covers packages without `main`.
    //
    // NO "node" condition: turbo-test runs in a browser-like V8 (turbo-dom, esbuild
    // --platform=browser), and packages' `node` builds target the real Node runtime — they pull
    // node-only APIs and, worse, TOP-LEVEL AWAIT (e.g. lexical's *.node.mjs does
    // `const mod = await import(...)`), which esbuild cannot emit as --format=cjs. That bundle
    // then fails and falls back to a raw-.mjs CJS compile → "Unexpected token 'export'" load
    // errors. Dropping `node` makes the resolver pick the package's `default`/`browser` build,
    // which is the right target for this env and CJS-compiles cleanly.
    //
    // `development` is a condition (turbo-test#211): vitest runs in development mode and matches
    // it, for an import and a require() alike (@emotion/sheet's development build).
    //
    // `require` is never a condition of an import (turbo-test#200): oxc matches the package's keys
    // in ITS order, so `{ require, default }` would pick the CJS build where Node/vitest load
    // `default`. Only an import that resolves nothing retries with `require` appended
    // (`require_fallback`) so require-only packages still load; a require() that resolves nothing
    // retries as an import (`resolve_spec_as_uncached`).
    //
    // A jest/node backend (NestJS, sequelize, ts-jest) is a CommonJS world: packages are
    // `require()`d, so their `require`/`node`/`default` export condition is the build that works
    // (sequelize's `lib/index.mjs` ESM build, picked by the `import` condition, loses
    // `DataTypes.ENUM` once bundled to CJS; `lib/index.js` is correct). Resolve CJS-first for such
    // projects, imports included. `node`/`default` cover packages that only ship ESM, and a
    // CJS-first bundle of an ESM file still works (esbuild ESM->CJS).
    // Condition matching is by the package's own key order against this SET — so to make `require`
    // win we must OMIT `import`/`module`/`browser`, not merely reorder. `node`/`default` still
    // cover packages without a `require` condition.
    let (conditions, mains): (&[&str], &[&str]) = if CJS_FIRST_RESOLUTION.with(|c| c.get()) {
        (
            &["require", "node", "default"],
            &["main", "module", "browser"],
        )
    } else if esm && require_fallback {
        (
            &["development", "import", "browser", "default", "require"],
            &["browser", "main", "module"],
        )
    } else if esm {
        (
            &["development", "import", "browser", "default"],
            &["browser", "main", "module"],
        )
    } else {
        (
            &["development", "require", "browser", "default"],
            &["browser", "main", "module"],
        )
    };
    oxc_resolver::ResolveOptions {
        extensions: strs(&[
            ".ts", ".tsx", ".mjs", ".js", ".jsx", ".cjs", ".json", ".node",
        ]),
        condition_names: strs(conditions),
        main_fields: strs(mains),
        // vitest/vite `resolve.alias` + `test.alias` (launcher → TURBO_ALIAS). oxc matches an alias
        // key exactly or as a `key/`-prefixed path, so `@` -> /abs/src turns `@/foo` into
        // /abs/src/foo (and leaves `@scope/pkg` alone). tsconfig `paths` stay separate (below).
        alias: config_alias_list(),
        // tsconfig `paths` aliases (e.g. "@/*" -> "./src/*"): the module-runner keeps
        // `require("@/...")` in transformed output (esbuild transform doesn't apply paths), so
        // the resolver must. Manual with the nearest tsconfig (Auto discovers from cwd, wrong).
        tsconfig: tsconfig.map(|config_file| {
            oxc_resolver::TsconfigDiscovery::Manual(oxc_resolver::TsconfigOptions {
                config_file,
                references: oxc_resolver::TsconfigReferences::Auto,
            })
        }),
        ..Default::default()
    }
}

/// Nearest tsconfig.json walking up from `dir` (so `paths` aliases resolve per project).
fn nearest_tsconfig(dir: &Path) -> Option<PathBuf> {
    // E12: memoize per-dir. Pure function of the filesystem layout, stable for a whole run, but
    // called on EVERY resolve (resolver_for) and re-walks the tree with is_file() syscalls each
    // time. A thread-local cache turns repeat lookups for the same dir into a hashmap hit.
    if e12_enabled() {
        thread_local! {
            static TSC_MEMO: RefCell<HashMap<PathBuf, Option<PathBuf>>> = RefCell::new(HashMap::new());
        }
        if let Some(v) = TSC_MEMO.with(|m| m.borrow().get(dir).cloned()) {
            return v;
        }
        let v = nearest_tsconfig_uncached(dir);
        TSC_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v.clone()));
        return v;
    }
    nearest_tsconfig_uncached(dir)
}

fn nearest_tsconfig_uncached(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(cur) = d {
        let tc = cur.join("tsconfig.json");
        if tc.is_file() {
            return Some(tc);
        }
        if cur.join("package.json").is_file()
            && cur
                .file_name()
                .map(|n| n == "node_modules")
                .unwrap_or(false)
        {
            break;
        }
        d = cur.parent();
    }
    None
}

/// Is `abs` an ES module (so its imports resolve with the "import" condition)? .mjs/.ts/.tsx are
/// ESM; .cjs is CJS; .js follows the nearest package.json "type".
fn is_esm_module(abs: &Path) -> bool {
    // E12: memoize per-file. Pure function of the filesystem layout, stable for a whole run, but
    // called on every module load (native_require) and re-reads package.json up the tree each time.
    if e12_enabled() {
        thread_local! {
            static ESM_MEMO: RefCell<HashMap<PathBuf, bool>> = RefCell::new(HashMap::new());
        }
        if let Some(v) = ESM_MEMO.with(|m| m.borrow().get(abs).copied()) {
            return v;
        }
        let v = is_esm_module_uncached(abs);
        ESM_MEMO.with(|m| m.borrow_mut().insert(abs.to_path_buf(), v));
        return v;
    }
    is_esm_module_uncached(abs)
}

fn is_esm_module_uncached(abs: &Path) -> bool {
    match abs.extension().and_then(|e| e.to_str()) {
        Some("mjs" | "mts" | "ts" | "tsx" | "jsx") => true,
        Some("cjs" | "cts") => false,
        _ => {
            let mut d = abs.parent();
            while let Some(dir) = d {
                if let Ok(s) = std::fs::read_to_string(dir.join("package.json")) {
                    return s.contains("\"type\": \"module\"") || s.contains("\"type\":\"module\"");
                }
                d = dir.parent();
            }
            false
        }
    }
}

/// The resolvers by (nearest tsconfig, ESM importer, CJS-first resolution).
type ResolverCache =
    HashMap<(Option<PathBuf>, bool, bool, bool), std::rc::Rc<oxc_resolver::Resolver>>;

/// A resolver configured for the nearest tsconfig of `from_dir` + the importer's module kind
/// (ESM vs CJS conditions), cached per (tsconfig, esm).
fn resolver_for(from_dir: &Path, esm: bool) -> std::rc::Rc<oxc_resolver::Resolver> {
    resolver_with(from_dir, esm, false)
}

/// `resolver_for`, optionally with the import conditions' `require` fallback (turbo-test#200).
fn resolver_with(
    from_dir: &Path,
    esm: bool,
    require_fallback: bool,
) -> std::rc::Rc<oxc_resolver::Resolver> {
    thread_local! {
        static RESOLVERS: RefCell<ResolverCache> = RefCell::new(HashMap::new());
    }
    // CJS-first mode is part of the key so a flip rebuilds the resolver (it changes conditions).
    let key = (
        nearest_tsconfig(from_dir),
        esm,
        CJS_FIRST_RESOLUTION.with(|c| c.get()),
        require_fallback,
    );
    RESOLVERS.with(|m| {
        if let Some(r) = m.borrow().get(&key) {
            return r.clone();
        }
        let r = std::rc::Rc::new(oxc_resolver::Resolver::new(base_resolve_options(
            key.0.clone(),
            esm,
            require_fallback,
        )));
        m.borrow_mut().insert(key, r.clone());
        r
    })
}

/// Decide CJS-first resolution for a project: true when the nearest config is a jest config with a
/// node (non-DOM) test environment and there's no vitest config — i.e. a CommonJS backend. Walks
/// up from `entry`; a vitest config short-circuits to ESM-first.
fn cjs_first_project(entry: &Path) -> bool {
    // E12: memoize keyed by the start dir (entry.parent()). The result is a pure function of the
    // directory's ancestor chain — NOT the filename — so every test file in a directory shares one
    // answer, yet each file otherwise re-walks up reading ~11 config names + package.json per dir.
    if e12_enabled() {
        if let Some(dir) = entry.parent() {
            thread_local! {
                static CJS_MEMO: RefCell<HashMap<PathBuf, bool>> = RefCell::new(HashMap::new());
            }
            if let Some(v) = CJS_MEMO.with(|m| m.borrow().get(dir).copied()) {
                return v;
            }
            let v = cjs_first_project_uncached(entry);
            CJS_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v));
            return v;
        }
    }
    cjs_first_project_uncached(entry)
}

fn cjs_first_project_uncached(entry: &Path) -> bool {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        for v in VITEST_CONFIG_FILES {
            if d.join(v).is_file() {
                return false; // vitest project → ESM-first
            }
        }
        let jest_cfg = [
            "jest.config.js",
            "jest.config.cjs",
            "jest.config.mjs",
            "jest.config.ts",
            "jest.config.json",
        ]
        .iter()
        .map(|n| d.join(n))
        .find(|p| p.is_file());
        let pkg_jest = std::fs::read_to_string(d.join("package.json"))
            .ok()
            .and_then(|s| package_json_jest_block(&s));
        if let Some(text) = jest_cfg
            .and_then(|p| std::fs::read_to_string(p).ok())
            .or(pkg_jest)
        {
            // node test env (jest's default) unless it explicitly asks for a DOM env.
            let env = config_string_value(&text, "testEnvironment");
            return !matches!(
                env.as_deref(),
                Some("jsdom") | Some("happy-dom") | Some("jest-environment-jsdom")
            );
        }
        dir = d.parent();
    }
    false
}

/// Nearest package.json `"type"` for a file (Node's module-determination rule).
fn nearest_pkg_type(path: &Path) -> Option<&'static str> {
    // E12: memoize keyed by the start dir (path.parent()). The nearest package.json "type" is a
    // pure function of the ancestor chain — NOT the filename — so all files/modules in a directory
    // share one answer; called on every module load (detect_kind), re-reading package.json up-tree.
    if e12_enabled() {
        if let Some(dir) = path.parent() {
            thread_local! {
                static PKG_TYPE_MEMO: RefCell<HashMap<PathBuf, Option<&'static str>>> = RefCell::new(HashMap::new());
            }
            if let Some(v) = PKG_TYPE_MEMO.with(|m| m.borrow().get(dir).copied()) {
                return v;
            }
            let v = nearest_pkg_type_uncached(path);
            PKG_TYPE_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v));
            return v;
        }
    }
    nearest_pkg_type_uncached(path)
}

fn nearest_pkg_type_uncached(path: &Path) -> Option<&'static str> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        let pj = d.join("package.json");
        if pj.is_file() {
            let s = std::fs::read_to_string(&pj).unwrap_or_default();
            if s.contains("\"type\": \"module\"") || s.contains("\"type\":\"module\"") {
                return Some("module");
            }
            if s.contains("\"type\": \"commonjs\"") || s.contains("\"type\":\"commonjs\"") {
                return Some("commonjs");
            }
            return None; // package.json present, no explicit type => CJS default by ext
        }
        dir = d.parent();
    }
    None
}

/// Kind for a resolved file. `.mjs`/`.ts`/`.tsx` are ESM, `.cjs` is CJS; ambiguous `.js`
/// follows the nearest package.json `"type"` (Node's rule), falling back to Node's syntax
/// detection: ES module syntax makes it ESM, anything else is CommonJS (turbo-test#79 item 5:
/// a comment mentioning "import " no longer makes tsc's CJS output an ES module).
fn detect_kind(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()) {
        Some("cjs") => Kind::Cjs,
        Some("mjs" | "mts" | "ts" | "tsx" | "jsx") => Kind::Esm,
        _ => match nearest_pkg_type(path) {
            Some("module") => Kind::Esm,
            Some("commonjs") => Kind::Cjs,
            _ => {
                let src = std::fs::read_to_string(path).unwrap_or_default();
                if crate::transform::has_module_syntax(&src) {
                    Kind::Esm
                } else {
                    Kind::Cjs
                }
            }
        },
    }
}

pub fn resolve_spec(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    resolve_spec_as(spec, from_dir, true)
}

/// The resolutions by (specifier, importer dir, ESM importer, CJS-first resolution).
type ResolveMemo = HashMap<(String, PathBuf, bool, bool), Option<PathBuf>>;

/// Resolve with the importer's module kind (`esm`) selecting ESM vs CJS export conditions.
pub fn resolve_spec_as(spec: &str, from_dir: &Path, esm: bool) -> Option<PathBuf> {
    // E12: memoize per (spec, from_dir, esm, cjs-first). native_require repeats the same
    // specifier from the same importer dir across every module/file that imports it; the oxc
    // resolve + canonicalize is deterministic for a run. cjs-first is in the key because it
    // flips the resolver conditions (mirrors resolver_for's own cache key).
    if e12_enabled() {
        thread_local! {
            static RES_MEMO: RefCell<ResolveMemo> = RefCell::new(HashMap::new());
        }
        let cjs = CJS_FIRST_RESOLUTION.with(|c| c.get());
        let key = (spec.to_string(), from_dir.to_path_buf(), esm, cjs);
        if let Some(v) = RES_MEMO.with(|m| m.borrow().get(&key).cloned()) {
            return v;
        }
        let v = resolve_spec_as_uncached(spec, from_dir, esm);
        RES_MEMO.with(|m| m.borrow_mut().insert(key, v.clone()));
        return v;
    }
    resolve_spec_as_uncached(spec, from_dir, esm)
}

fn resolve_spec_as_uncached(spec: &str, from_dir: &Path, esm: bool) -> Option<PathBuf> {
    // Vite/plugin query suffixes (`./icon.svg?react`, `./x.svg?url`, `./y?raw`). oxc can't resolve a
    // spec with a `?query`, so peel it off. `?react` (vite-plugin-svgr) is special: it yields a React
    // component — resolve to a shared render-safe SVGR stub. Every other query resolves the base file
    // (its content is served by the asset loader / text), so the import never hard-errors.
    if let Some(qpos) = spec.find('?') {
        let (base, query) = (&spec[..qpos], &spec[qpos + 1..]);
        // svgr: the `react` query flag (`?react`, `?react&foo`) — a bare flag token split on `&`,
        // NOT a value like `?foo=react`. `.svg` extension required.
        if base.ends_with(".svg") && query.split('&').any(|t| t == "react") {
            return svgr_stub_path(from_dir);
        }
        return resolve_spec_as_uncached(base, from_dir, esm);
    }
    // A `node_modules/<pkg>/...` import (some tests `await import('node_modules/x/dist/y')`)
    // is really a bare specifier — strip the prefix so oxc resolves it from node_modules.
    let spec = spec.strip_prefix("node_modules/").unwrap_or(spec);
    // Unified Node resolution (relative + bare) via oxc_resolver — handles package.json
    // main/exports/imports, conditions, index files, and extensions uniformly. This is
    // what makes deep node_modules graphs (@mui internal "./styles" etc.) resolve.
    // a require() of a package with no `require` / `default` build: as an import (an ES-only
    // package, which Node's require(esm) loads)
    let resolved = resolver_for(from_dir, esm)
        .resolve(from_dir, spec)
        .or_else(|e| {
            if esm {
                // an import of a require-only package (no import/default build): its `require`
                resolver_with(from_dir, true, true)
                    .resolve(from_dir, spec)
                    .map_err(|_| e)
            } else {
                resolver_for(from_dir, true).resolve(from_dir, spec)
            }
        });
    if let Ok(res) = resolved {
        if let Ok(c) = crate::canon::canonicalize(res.path()) {
            // oxc can resolve a bare `require("..")` / `require(".")` (a directory re-export, common
            // in NestJS package `index.js` → `require("./dist")` chains and `dist/utils/x.js →
            // require("..")` self-references) to the DIRECTORY path rather than its index file. A
            // directory isn't loadable as a module → fall through to the manual index probing below
            // instead of returning the dir (which threw "cannot resolve ..").
            if c.is_file() {
                return Some(c);
            }
        }
    }
    // fallback: manual probing for local files oxc may skip (incl. bare "." / ".." directory specs)
    if spec.starts_with('.') || spec.starts_with('/') {
        return probe_local_file(from_dir, spec);
    }
    None
}

/// Try each extension / index.* form that a Node-style local import may actually mean, returning
/// the first file that exists (canonicalized). Only used for `./` / `/` specs.
fn probe_local_file(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    let base = from_dir.join(spec);
    for c in [
        base.clone(),
        base.with_extension("mjs"),
        base.with_extension("js"),
        base.with_extension("mts"),
        base.with_extension("ts"),
        base.with_extension("tsx"),
        base.with_extension("cjs"),
        base.join("index.mjs"),
        base.join("index.js"),
        base.join("index.ts"),
        base.join("index.tsx"),
        base.join("index.jsx"),
        base.join("index.cjs"),
    ] {
        if c.is_file() {
            return crate::canon::canonicalize(c).ok();
        }
    }
    None
}

/// A vite-plugin-svgr stub: a `./x.svg?react` import resolves here. A `forwardRef` React component
/// that renders a real `<svg>` with the caller's props AND ref forwarded — enough for the
/// near-universal test patterns (render it, query by role/testid, assert a forwarded ref) without
/// svgr's real SVG→JSX transform. It builds the element with React's OWN `createElement`, so the
/// element carries the version-correct `$$typeof` (React 18 vs 19 differ) and refs work — a
/// hand-built element would hard-code one React era's symbol and silently drop refs.
///
/// `react` is imported by the CONCRETE PATH resolved from the importer (`from_dir`), not bare — the
/// stub lives in the cache dir (no `node_modules`), and resolving react per-importer + keying the
/// stub file by that path means every importer binds to exactly the react instance it would itself
/// resolve (monorepo-safe: two packages with different react copies get two stub files, each bound
/// correctly — no shared last-writer-wins state). Falls back to a bare `react` import when react
/// can't be resolved. The filename also hashes the stub body, so an upgraded turbo-test never reuses
/// a stale stub from the shared temp cache. `default` and legacy `ReactComponent` are the same
/// component (svgr exports both).
fn svgr_stub_path(from_dir: &Path) -> Option<PathBuf> {
    // Import react by the exact path this importer resolves (so the stub shares the one instance);
    // fall back to bare `react`. Single-quote + escape for embedding in the generated JS.
    let react_spec = resolve_spec("react", from_dir)
        .map(|p| {
            let esc = p
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029");
            format!("'{esc}'")
        })
        .unwrap_or_else(|| "'react'".to_string());
    let src = SVGR_STUB_TEMPLATE.replace("__REACT_SPEC__", &react_spec);
    // Name the file by a hash of its CONTENT (which includes the resolved react path) so distinct
    // react instances get distinct stubs and a changed stub never collides with a stale one.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let path = cache_dir().join(format!("__turbo_svgr_stub_{:016x}.mjs", h.finish()));
    if !path.exists() {
        write_atomic(&path, &src);
    }
    crate::canon::canonicalize(&path).ok()
}

/// Body of the svgr `?react` stub; `__REACT_SPEC__` is replaced with the quoted react import spec.
const SVGR_STUB_TEMPLATE: &str = r#"// turbo-test: vite-plugin-svgr `?react` mock — a render-safe <svg> component.
import { createElement, forwardRef } from __REACT_SPEC__;
const SvgrMock = forwardRef(function SvgrMock(props, ref) {
  var p = {};
  if (props) { for (var k in props) { if (k !== 'ref' && k !== 'key') p[k] = props[k]; } }
  if (p['aria-hidden'] === undefined && p['aria-label'] === undefined && p.title === undefined && p.role === undefined) {
    p['aria-hidden'] = true;
  }
  if (ref !== undefined && ref !== null) p.ref = ref;
  return createElement('svg', p);
});
export default SvgrMock;
export { SvgrMock as ReactComponent };
"#;

/// Non-JS assets that Vite/Vitest stub: importing them must not crash the graph.
fn is_asset(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(
            "css"
                | "scss"
                | "sass"
                | "less"
                | "styl"
                | "svg"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "avif"
                | "ico"
                | "woff"
                | "woff2"
                | "ttf"
                | "eot"
                | "otf"
                | "mp4"
                | "webm"
                | "wav"
                | "mp3"
                | "txt"
        )
    )
}

/// The origin of a CommonJS module wrapper (a classic script) named `name`: as
/// [`module_origin`]'s, not a module.
fn script_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        123,
        None,
        false,
        false,
        false,
        None,
    )
}

fn module_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        123,
        None,
        false,
        false,
        true,
        None,
    )
}

/// CJS modules get `__dirname`/`__filename` as wrapper-function params (see `load_cjs_inner`); a
/// real ES module (the `load_graph` path) has neither in scope, so an app `.ts` file that reads
/// `const COUNTRIES_DIR = __dirname` at top level throws `ReferenceError: __dirname is not defined`.
/// Node's ESM doesn't expose these, but our graph path loads app code authored for the CJS world —
/// so we inject module-local `const` bindings matching Node's CJS semantics (`__filename` = the
/// file, `__dirname` = its directory).
///
/// Injected only when the (already-transformed) body actually references the identifier and does
/// not declare it itself — so esbuild bundles that already shim `__dirname`, or files that bind it
/// locally, are left untouched (avoids a redeclaration `SyntaxError`). The prelude carries no
/// newline, so existing line numbers (stack traces / source maps) are preserved.
fn inject_dirname_esm(abs: &Path, body: &str) -> Option<String> {
    let needs_dir = references_ident(body, "__dirname") && !declares_ident(body, "__dirname");
    let needs_file = references_ident(body, "__filename") && !declares_ident(body, "__filename");
    if !needs_dir && !needs_file {
        return None;
    }
    let mut prelude = String::new();
    if needs_file {
        let file = js_string_literal(&abs.to_string_lossy());
        prelude.push_str(&format!("const __filename={file};"));
    }
    if needs_dir {
        let dir = abs.parent().unwrap_or_else(|| Path::new("/"));
        let dir = js_string_literal(&dir.to_string_lossy());
        prelude.push_str(&format!("const __dirname={dir};"));
    }
    Some(format!("{prelude}{body}"))
}

/// `ident` appears as a whole-word token (not as a substring of a longer identifier and not in the
/// `foo.__dirname` member-access position). Cheap heuristic — good enough to gate the injection.
fn references_ident(src: &str, ident: &str) -> bool {
    let bytes = src.as_bytes();
    let id_len = ident.len();
    let mut i = 0;
    while let Some(rel) = src.get(i..).and_then(|s| s.find(ident)) {
        let at = i + rel;
        let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
        let after = at + id_len;
        let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        // skip `.`-prefixed member access (`x.__dirname`) — that's a property, not the global.
        let not_member = at == 0 || bytes[at - 1] != b'.';
        if before_ok && after_ok && not_member {
            return true;
        }
        i = at + id_len;
    }
    false
}

/// The body declares `ident` via `const`/`let`/`var` at any position (so we must not re-inject).
fn declares_ident(src: &str, ident: &str) -> bool {
    for kw in ["const ", "let ", "var "] {
        let needle = format!("{kw}{ident}");
        let mut i = 0;
        while let Some(rel) = src.get(i..).and_then(|s| s.find(&needle)) {
            let at = i + rel + needle.len();
            // next char must end the identifier (`=`, space, `,`, `;`, etc.) — not extend it.
            if at >= src.len() || !is_ident_byte(src.as_bytes()[at]) {
                return true;
            }
            i = at;
        }
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Build a double-quoted JS string literal for `s` (escapes the metacharacters that appear in
/// filesystem paths). Used to embed `__dirname`/`__filename` path constants in injected source.
pub(crate) fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

use std::sync::atomic::{AtomicU64, Ordering};
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static ATOMIC_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write a cache file atomically (temp + rename) so parallel workers never read a half-written
/// transform — a same-content content-addressed write racing across jobs otherwise corrupts it.
fn write_atomic(path: &Path, content: &str) {
    let seq = ATOMIC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{}", std::process::id(), seq));
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Cache hit-rate over this process (warm-run KPI, spec §8 >90%).
pub fn transform_cache_stats() -> (u64, u64) {
    (
        CACHE_HITS.load(Ordering::Relaxed),
        CACHE_MISSES.load(Ordering::Relaxed),
    )
}

const TRANSFORM_VERSION: &str = "oxc-0.134-v1";

/// The default transform cache under `tmp`: one directory per turbo-test build
/// (`turbo-test-cache/<version>-<build id>`, build.rs), so an entry another build wrote (an older
/// release, a pre-release, another worktree) is never served as a hit (turbo-test#207).
fn default_cache_dir(tmp: &Path) -> PathBuf {
    tmp.join("turbo-test-cache").join(format!(
        "{}-{}",
        env!("CARGO_PKG_VERSION"),
        env!("TT_BUILD_ID")
    ))
}

fn cache_dir() -> PathBuf {
    // TURBO_CACHE_DIR overrides the shared temp cache — lets a test run against an isolated,
    // freshly-cleared cache without disturbing other concurrent turbo-test processes that share
    // the default location.
    let d = std::env::var_os("TURBO_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_cache_dir(&std::env::temp_dir()));
    // create_dir_all runs on every call (9 call sites per module load) though the dir only needs
    // creating once. Memoize the (dir -> created) fact in a OnceLock keyed by the resolved path so
    // the syscall fires exactly once per distinct cache dir for the life of the process.
    static ENSURED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    let ensured = ENSURED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut set = ensured.lock().unwrap();
    if set.insert(d.clone()) {
        let _ = std::fs::create_dir_all(&d);
    }
    d
}

/// Content-addressed key: hash(source + extension + transform-tool version).
/// A change in any input misses the cache; identical inputs hit across runs/workers.
fn cache_key(abs: &Path, src: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    abs.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .hash(&mut h);
    TRANSFORM_VERSION.hash(&mut h);
    h.finish()
}

/// Nearest project root containing esbuild (for dep bundling).
fn project_root(file: &Path) -> Option<PathBuf> {
    // E12: memoize keyed by the start dir (file.parent()). The esbuild project root is a pure
    // function of the ancestor chain — NOT the filename — so all files/modules in a directory share
    // one answer; called from many load paths, each re-walking with an exists() syscall per dir.
    if e12_enabled() {
        if let Some(dir) = file.parent() {
            thread_local! {
                static ROOT_MEMO: RefCell<HashMap<PathBuf, Option<PathBuf>>> = RefCell::new(HashMap::new());
            }
            if let Some(v) = ROOT_MEMO.with(|m| m.borrow().get(dir).cloned()) {
                return v;
            }
            let v = project_root_uncached(file);
            ROOT_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v.clone()));
            return v;
        }
    }
    project_root_uncached(file)
}

fn project_root_uncached(file: &Path) -> Option<PathBuf> {
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir.join("node_modules/.bin/esbuild").exists() {
            return Some(dir.to_path_buf());
        }
        d = dir.parent();
    }
    // No project-local esbuild: pnpm keeps a transitive esbuild under node_modules/.pnpm and links
    // no node_modules/.bin/esbuild, so every vi.mock factory used to fall back to raw (untransformed)
    // evaluation. With turbo-test's own esbuild available (TURBO_ESBUILD, set by cli.js), the nearest
    // ancestor that has node_modules is the root: bare imports resolve from there and the prepass
    // writes into its node_modules.
    if std::env::var_os("TURBO_ESBUILD").is_some() {
        let mut d = file.parent();
        while let Some(dir) = d {
            if dir.join("node_modules").is_dir() {
                return Some(dir.to_path_buf());
            }
            d = dir.parent();
        }
    }
    None
}

/// The esbuild binary for `root`: the project's own `node_modules/.bin/esbuild` when it has one,
/// otherwise turbo-test's bundled esbuild (`TURBO_ESBUILD`, resolved by cli.js).
fn esbuild_bin(root: &Path) -> PathBuf {
    let local = root.join("node_modules/.bin/esbuild");
    if local.exists() {
        return local;
    }
    match std::env::var_os("TURBO_ESBUILD") {
        Some(p) => PathBuf::from(p),
        None => local,
    }
}

/// Nearest ancestor with `node_modules/typescript` — the root `tsc_transform_esm` needs. Unlike
/// `project_root` this does NOT require esbuild (ts-jest backends often ship typescript but not
/// esbuild), so the emitDecoratorMetadata→tsc routing fires for them.
fn tsc_root(file: &Path) -> Option<PathBuf> {
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir.join("node_modules/typescript/package.json").exists() {
            return Some(dir.to_path_buf());
        }
        d = dir.parent();
    }
    None
}

/// Bundle a test file (and ALL its node_modules deps) into one clean ESM file via esbuild
/// — the Vite approach. This is what makes @mui / @testing-library / react-dom / emotion etc.
/// load reliably instead of hand-rolling CJS/ESM interop across the whole dep tree. `vitest`
/// is externalized (we provide it); CSS/assets get stub loaders. Returns the bundled path,
/// or None to fall back to the native per-module loader. Cached by source path + mtime.
///
/// Bundle a test/setup file. `externals` are vi.mock specifiers kept OUT of the bundle so the
/// native loader (+ mock registry) intercepts them; after bundling they are rewritten from
/// their (file-relative) form to the resolved ABSOLUTE path so they resolve correctly from
/// the bundle's cache location and match the mock registered under the same absolute path.
fn esbuild_bundle(file: &Path, externals: &[String]) -> Option<PathBuf> {
    esbuild_bundle_full(file, externals, &std::collections::HashMap::new(), None)
}

/// last path segment of a specifier (the module "basename"), e.g. "../../x/analytics" -> "analytics".
fn spec_basename(spec: &str) -> String {
    spec.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(spec)
        .to_string()
}

/// Bundle with externalized mock specifiers, rewriting each externalized import to the single
/// absolute mock target for its basename (all depth-variants of a relative mock specifier
/// point at the same module). `root_override` forces the node_modules base for generated files.
fn esbuild_bundle_full(
    file: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
    root_override: Option<&Path>,
) -> Option<PathBuf> {
    if std::env::var("TURBO_NO_BUNDLE").is_ok() {
        return None;
    }
    let root = match root_override {
        Some(r) => r.to_path_buf(),
        None => project_root(file)?,
    };
    let esbuild = esbuild_bin(&root);
    let out = bundle_full_cache_path(file, externals, rewrite_map);
    // Record the source dir so resolve_callback can resolve this bundle's externalized bare
    // imports (the cache file has no node_modules of its own).
    if let Some(src_dir) = file.parent() {
        REGISTRY.with(|r| {
            r.borrow_mut()
                .bundle_src_dir
                .insert(out.clone(), src_dir.to_path_buf())
        });
    }
    if out.exists() {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(out);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    // tsconfig (for path aliases like "@/*") — esbuild resolves them when pointed at it.
    let tsconfig = find_tsconfig_up_to(file, &root);
    esbuild_bundle_full_build(
        file,
        externals,
        rewrite_map,
        &root,
        &esbuild,
        &out,
        tsconfig,
    )
}

/// Content-addressed cache path for `esbuild_bundle_full` (file, mtime, externals, rewrites).
fn bundle_full_cache_path(
    file: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mtime = std::fs::metadata(file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut h);
    mtime.hash(&mut h);
    for e in externals {
        e.hash(&mut h);
    }
    let mut rw: Vec<_> = rewrite_map.iter().collect();
    rw.sort();
    for (k, v) in rw {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    "esb-v5-import-rewrite".hash(&mut h);
    mr_enabled().hash(&mut h);
    cache_dir().join(format!("esb-{:016x}.mjs", h.finish()))
}

/// Nearest `tsconfig.json` from `file`'s dir up to (and including) `root`.
fn find_tsconfig_up_to(file: &Path, root: &Path) -> Option<PathBuf> {
    let mut d = file.parent();
    while let Some(dir) = d {
        let tc = dir.join("tsconfig.json");
        if tc.is_file() {
            return Some(tc);
        }
        if dir == root {
            break;
        }
        d = dir.parent();
    }
    None
}

/// Run esbuild for `esbuild_bundle_full` into a temp file, rewrite mock imports, publish to `out`.
fn esbuild_bundle_full_build(
    file: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
    root: &Path,
    esbuild: &Path,
    out: &Path,
    tsconfig: Option<PathBuf>,
) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new(esbuild);
    cmd.current_dir(root).arg(file).args([
        "--bundle",
        "--format=esm",
        "--platform=browser",
        "--jsx=automatic",
        "--external:vitest",
        "--loader:.css=empty",
        "--loader:.scss=empty",
        "--loader:.sass=empty",
        "--loader:.less=empty",
        "--loader:.svg=text",
        "--loader:.png=dataurl",
        "--loader:.jpg=dataurl",
        "--loader:.jpeg=dataurl",
        "--loader:.gif=dataurl",
        "--loader:.webp=dataurl",
        "--log-level=silent",
        "--define:process.env.NODE_ENV=globalThis.process.env.NODE_ENV",
    ]);
    for e in externals {
        cmd.arg(format!("--external:{e}"));
    }
    // Under the module-runner, this ESM bundle (setup-file fallback) must NOT inline node_modules
    // packages: the entry + its require-graph load every package via the shared CJS dep cache
    // (esbuild_bundle_dep_cjs uses --packages=external), so any package this bundle inlines
    // becomes a SECOND, separate instance. For a module-level singleton (react's dispatcher,
    // an emotion cache, a MUI theme context, a react-query client) that split silently breaks
    // things — the classic symptom is a setup-bundled component rendering against a second react
    // ("Invalid hook call / Cannot read properties of null (reading 'useRef')"). Externalize ALL
    // packages (generic — no library names) so each bare import resolves via resolve_callback ->
    // native_require to the ONE shared instance, exactly like the CJS dep-bundle path. The bundle
    // then carries only this file's own (relative) code; resolve_callback resolves the externals
    // from the bundle's recorded source dir (registered below) and lazy-loads them on demand.
    if mr_enabled() {
        cmd.arg("--packages=external");
    }
    if let Some(tc) = &tsconfig {
        cmd.arg(format!("--tsconfig={}", tc.display()));
    }
    // Write to a UNIQUE temp outfile, then atomically rename into the content-addressed cache
    // path. esbuild's --outfile writes NON-atomically (it truncates+streams to the target), so
    // pointing it straight at `out` lets a concurrent worker — which only checks `out.exists()`
    // (above) — read a half-written / empty bundle. An empty setup bundle silently imports
    // nothing, so `setupFiles`' `expect.extend` never runs and that whole worker loses its custom
    // matchers for every file (the jest-dom "toBeInTheDocument is not a function" cascade). Every
    // OTHER esbuild cache path already goes through write_atomic for this exact reason; this one
    // used --outfile directly. Mirror the temp+rename here.
    let seq = ATOMIC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = out.with_extension(format!("tmp{}-{}.mjs", std::process::id(), seq));
    cmd.arg(format!("--outfile={}", tmp.display()))
        .stdout(std::process::Stdio::null());
    if std::env::var("TURBO_ESBUILD_DEBUG").is_err() {
        cmd.stderr(std::process::Stdio::null());
    }
    let status = cmd.status().ok();
    if !status.map(|s| s.success()).unwrap_or(false) || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    // Rewrite every externalized mock import to the single absolute mock target for its
    // basename. All depth-variants of a relative mock specifier ("../analytics",
    // "../../../analytics", ...) point at the same module, so they map to one abs path that
    // matches the mock registered under that path — regardless of which nested module wrote
    // the import (whose depth differs from the entry's). Done on the temp file, before publish.
    if !externals.is_empty() && !rewrite_map.is_empty() {
        rewrite_mock_imports_in(&tmp, externals, rewrite_map);
    }
    // Publish atomically: `out` only ever becomes visible fully-written. A racing worker either
    // sees no `out` (and builds its own temp) or the complete file — never a partial one.
    publish_bundle(&tmp, out).then(|| out.to_path_buf())
}

/// Rewrite the externalized mock import specifiers in the bundle at `tmp` to their abs targets.
fn rewrite_mock_imports_in(
    tmp: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
) {
    if let Ok(mut text) = std::fs::read_to_string(tmp) {
        let mut changed = false;
        for spec in externals {
            if let Some(abs) = rewrite_map.get(&spec_basename(spec)) {
                let abs_s = abs.to_string_lossy();
                // only the import specifiers esbuild writes (`from "x"`, `import "x"`,
                // `import("x")`): a string literal quoting the specifier (a `vi.mock` call's
                // argument, a message) keeps it (turbo-test#196)
                for (q, lead) in [('"', "from "), ('"', "import "), ('"', "import(")]
                    .into_iter()
                    .chain([('\'', "from "), ('\'', "import "), ('\'', "import(")])
                {
                    let from = format!("{lead}{q}{spec}{q}");
                    let to = format!("{lead}{q}{abs_s}{q}");
                    if text.contains(&from) {
                        text = text.replace(&from, &to);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            let _ = std::fs::write(tmp, &text);
        }
    }
}

/// Rename the finished temp bundle onto its cache path. Rename can fail if another worker already
/// published an identical `out`; fall back to it when present, else give up (and clean the temp).
/// Returns whether `out` is usable.
fn publish_bundle(tmp: &Path, out: &Path) -> bool {
    if std::fs::rename(tmp, out).is_err() {
        let _ = std::fs::remove_file(tmp);
        if !out.exists() {
            return false;
        }
    }
    true
}

/// Parse a per-file `// @vitest-environment <env>` pragma (vitest honors a docblock comment at
/// the top of a test file). Returns the lowercased environment name if present. We scan the
/// whole file (cheap) rather than only the leading docblock — a stray mention is unlikely and
/// matching vitest's exact docblock placement isn't worth the parse.
fn file_env_pragma(s: &str) -> Option<String> {
    let i = s.find("@vitest-environment")?;
    let rest = &s[i + "@vitest-environment".len()..];
    let name: String = rest
        .trim_start_matches([' ', ':', '\t'])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name.to_ascii_lowercase())
    }
}

/// Resolve the effective environment for a file: a per-file `@vitest-environment` pragma wins,
/// then the run-level `TURBO_ENV` (set by cli.js from `--environment` / config `test.environment`),
/// else `None` (auto — fall back to the content heuristic). Recognized: node | jsdom | happy-dom.
fn forced_env(s: &str) -> Option<String> {
    file_env_pragma(s).or_else(|| {
        std::env::var("TURBO_ENV")
            .ok()
            .map(|v| v.to_ascii_lowercase())
    })
}

thread_local! {
    /// This worker thread's pool slot (0-based) and the run-wide sequence number of the file it
    /// is running ([`set_worker_slot`]).
    static WORKER_SLOT: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Called by the worker pool before each file: `pool` is the worker's 0-based slot, `seq` the
/// file's position in the run (vitest's VITEST_POOL_ID / VITEST_WORKER_ID, [`set_file_env`]).
pub fn set_worker_slot(pool: usize, seq: usize) {
    WORKER_SLOT.with(|w| w.set((pool, seq)));
}

/// The per-file part of vitest's process.env (turbo-test#123, #156): vite's `SSR` (`''` under a
/// DOM environment — `jsdom` / `happy-dom`, configured or by pragma, vitest's web transform mode
/// — else `'1'`), `VITEST_POOL_ID` (the worker, from 1) and `VITEST_WORKER_ID` (vitest numbers a
/// fresh worker per file from 0; a reused isolate keeps its worker's). Also `expect.getState()`'s
/// `environment`.
fn set_file_env(scope: &mut v8::PinScope, file: &Path) {
    let s = std::fs::read_to_string(file).unwrap_or_default();
    let env = forced_env(&s);
    let web = env.as_deref().is_some_and(is_dom_env);
    let (pool, seq) = WORKER_SLOT.with(|w| w.get());
    let worker = if on_reuse_path() { pool } else { seq };
    run_js(
        scope,
        &format!(
            "globalThis.process.env.SSR = '{}'; globalThis.process.env.VITEST_POOL_ID = '{}'; \
             globalThis.process.env.VITEST_WORKER_ID = '{}'; globalThis.__ttEnvName = {};",
            if web { "" } else { "1" },
            pool + 1,
            worker,
            json_string(match env.as_deref() {
                Some("happydom") => "happy-dom",
                Some(e) => e,
                None => "node",
            }),
        ),
    );
}

/// Whether an environment name is a DOM one: vitest's `jsdom` / `happy-dom`, or `turbo-dom`, the
/// name of the built-in DOM itself (turbo-test#217) — each installs the DOM for every file.
fn is_dom_env(env: &str) -> bool {
    matches!(env, "jsdom" | "happy-dom" | "happydom" | "turbo-dom")
}

/// Whether a file needs a DOM environment (so we don't impose one on pure-logic tests,
/// which keeps their clean globals — mirrors vitest's per-file environment).
///
/// An explicit environment (per-file pragma or `TURBO_ENV`) is authoritative: `node` SKIPS the
/// DOM install even for a `.tsx` file; `jsdom`/`happy-dom` FORCE it on. With no explicit env we
/// fall back to the content heuristic (extension + DOM-API mentions).
fn needs_dom(file: &Path) -> bool {
    let s = std::fs::read_to_string(file).unwrap_or_default();
    match forced_env(&s).as_deref() {
        Some("node") => return false,
        // turbo-dom is our single DOM impl; jsdom/happy-dom both map onto it.
        Some(e) if is_dom_env(e) => return true,
        _ => {}
    }
    if matches!(
        file.extension().and_then(|e| e.to_str()),
        Some("tsx" | "jsx")
    ) {
        return true;
    }
    source_mentions_dom(&s)
}

/// Whether `s` mentions any of the DOM-API fingerprints that make the content heuristic flip on.
fn source_mentions_dom(s: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "@testing-library",
        "react-dom",
        "ReactDOM",
        "document",
        "window.",
        "getComputedStyle",
        "happy-dom",
        "jsdom",
        "@vitest-environment",
    ];
    NEEDLES.iter().any(|n| s.contains(n))
}

/// Whether the project asks for non-isolated runs (`isolate: false`) in any of its vitest
/// configs near `entry`. turbo-test then reuses one isolate per worker (vitest isolate:false
/// semantics) — node_modules barrels evaluate once, not per file. The env var TURBO_REUSE_ISOLATE
/// forces it on regardless; TURBO_NO_REUSE forces it off.
fn vitest_isolate_false(entry: &Path) -> bool {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        // any vitest config variant the project uses (dev/ci/coverage/shard commonly set isolate)
        let variants = ["vitest.config.dev.ts", "vitest.config.ci.ts"];
        for cfg in VITEST_CONFIG_FILES.iter().chain(&variants) {
            if let Ok(s) = std::fs::read_to_string(d.join(cfg)) {
                // strip `//` line comments first — a doc comment mentioning "isolate: false"
                // must not be mistaken for the real setting.
                let code: String = s
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");
                let flat: String = code.chars().filter(|c| !c.is_whitespace()).collect();
                if strip_pool_options(&flat).contains("isolate:false") {
                    return true;
                }
            }
        }
        // stop at the project root (nearest package.json outside node_modules)
        if d.join("package.json").is_file() && !d.to_string_lossy().contains("node_modules") {
            break;
        }
        dir = d.parent();
    }
    false
}

/// `flat` (a whitespace-free config) without its `poolOptions:{...}` blocks: vitest 4 removed
/// `test.poolOptions` (it only logs a deprecation) and keeps `isolate: true`, so an `isolate`
/// inside one is not the project's setting (turbo-test#215).
fn strip_pool_options(flat: &str) -> String {
    let mut out = String::with_capacity(flat.len());
    let mut rest = flat;
    while let Some(at) = rest.find("poolOptions:{") {
        out.push_str(&rest[..at]);
        let body = &rest[at + "poolOptions:".len()..];
        let mut depth = 0usize;
        let end = body
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(i + 1)
            })
            .unwrap_or(body.len());
        rest = &body[end..];
    }
    out.push_str(rest);
    out
}

static REUSE_DECISION: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The reuse decision for this run, computed once from env + the project's vitest config.
/// Called at the top of run_test_file (which has the entry path); cached for every later call.
fn reuse_decision(entry: &Path) -> bool {
    *REUSE_DECISION.get_or_init(|| {
        if std::env::var("TURBO_NO_REUSE").is_ok() {
            return false;
        }
        std::env::var("TURBO_REUSE_ISOLATE").is_ok() || vitest_isolate_false(entry)
    })
}

/// Find the project's vitest setupFiles (registers jest-dom matchers etc.) by parsing the
/// nearest vitest/vite config. Returns resolved absolute paths.
/// Find a config object key (e.g. `setupFiles`) — the occurrence actually used as a key
/// (`key:`, or the quoted JSON form `"key":` / `'key':`), skipping mentions inside `//`
/// comments or prose (e.g. a doc comment that names `setupFiles`) and longer names that merely
/// end in `key`. Returns the byte offset of the key name.
fn find_config_key(s: &str, key: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(i) = s[search..].find(key) {
        let at = search + i;
        search = at + key.len();
        let before = s[..at].chars().next_back();
        let mut after = &s[at + key.len()..];
        // a quoted key: the same quote must close it
        if let Some(q) = before.filter(|c| *c == '"' || *c == '\'') {
            match after.strip_prefix(q) {
                Some(rest) => after = rest,
                None => continue,
            }
        } else if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            continue; // `myjest:` is not `jest:`
        }
        // must be immediately followed (after optional whitespace) by ':'
        if !after.trim_start().starts_with(':') {
            continue;
        }
        // skip a key inside a `//` or `/* */` comment (strings and regexes start none): the
        // comment stripper then drops the key itself
        if !crate::launcher::strip_comments(&s[..at + key.len()]).ends_with(key) {
            continue;
        }
        return Some(at);
    }
    None
}

fn vitest_setup_files(entry: &Path) -> Vec<PathBuf> {
    // E12: memoize keyed by the start dir (entry.parent()). The setup-file list is a pure function
    // of the ancestor chain — NOT the filename (resolved relative to the config dir, not the entry)
    // — so every test file in a directory shares one answer; each otherwise re-walks configs up-tree.
    if e12_enabled() {
        if let Some(dir) = entry.parent() {
            thread_local! {
                static SETUP_MEMO: RefCell<HashMap<PathBuf, Vec<PathBuf>>> = RefCell::new(HashMap::new());
            }
            if let Some(v) = SETUP_MEMO.with(|m| m.borrow().get(dir).cloned()) {
                return v;
            }
            let v = vitest_setup_files_uncached(entry);
            SETUP_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v.clone()));
            return v;
        }
    }
    vitest_setup_files_uncached(entry)
}

use crate::launcher::VITEST_CONFIG_FILES;

fn vitest_setup_files_uncached(entry: &Path) -> Vec<PathBuf> {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        if let Some(out) = vitest_setup_files_in_dir(d) {
            return out;
        }
        // No vitest config in this dir — try a jest config (drop-in for jest projects).
        if let Some(found) = jest_setup_files(d) {
            return found;
        }
        dir = d.parent();
    }
    Vec::new()
}

/// `setupFiles` resolved against directory `d` if any vitest config lives here; `None` when none
/// does (so the caller keeps walking up). An empty vec means "config found, no setupFiles".
fn vitest_setup_files_in_dir(d: &Path) -> Option<Vec<PathBuf>> {
    for cfg in VITEST_CONFIG_FILES {
        let p = d.join(cfg);
        let Ok(s) = std::fs::read_to_string(&p) else {
            continue;
        };
        return Some(parse_setup_files_array(&s, d));
    }
    None
}

/// The `setupFiles: [...]` entries in `s`, each resolved against `d`; an empty vec means no
/// resolvable entries (or no `setupFiles` key at all — vitest's default).
fn parse_setup_files_array(s: &str, d: &Path) -> Vec<PathBuf> {
    let Some(start) = find_config_key(s, "setupFiles") else {
        return Vec::new();
    };
    let tail = &s[start..];
    let Some(lb) = tail.find('[') else {
        return Vec::new();
    };
    let Some(rb) = tail[lb..].find(']') else {
        return Vec::new();
    };
    let inner = &tail[lb + 1..lb + rb];
    let mut out = Vec::new();
    for part in inner.split(',') {
        let q = part
            .trim()
            .trim_matches(|c| c == '\'' || c == '"' || c == '`');
        if q.is_empty() {
            continue;
        }
        if let Some(abs) = resolve_spec(q, d) {
            out.push(abs);
        }
    }
    out
}

/// vitest's `snapshotSerializers` (turbo-test#167): the paths in the nearest vitest config,
/// resolved against its directory (vitest resolves them against the root), memoized by the
/// test file's directory as the setup files are.
fn vitest_snapshot_serializers(entry: &Path) -> Vec<PathBuf> {
    // (a test file's path always has a directory)
    let dir = entry.parent().unwrap_or(entry);
    thread_local! {
        static SERIALIZERS_MEMO: RefCell<HashMap<PathBuf, Vec<PathBuf>>> = RefCell::new(HashMap::new());
    }
    if let Some(v) = SERIALIZERS_MEMO.with(|m| m.borrow().get(dir).cloned()) {
        return v;
    }
    let v = vitest_snapshot_serializers_uncached(dir);
    SERIALIZERS_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v.clone()));
    v
}

fn vitest_snapshot_serializers_uncached(start: &Path) -> Vec<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for cfg in VITEST_CONFIG_FILES {
            if let Ok(s) = std::fs::read_to_string(d.join(cfg)) {
                return config_string_list(&s, "snapshotSerializers")
                    .into_iter()
                    .map(|f| normalize_path(&d.join(f)))
                    .collect();
            }
        }
        dir = d.parent();
    }
    Vec::new()
}

/// The parts of the nearest vitest config the runtime reads (`__ttApplyConfig`), as a JS object
/// literal, memoized by the test file's directory: `test.fakeTimers`' `{ ... }` source
/// (turbo-test#180) and the `clearMocks` / `mockReset` / `restoreMocks` / `unstubEnvs` /
/// `unstubGlobals` switches set to `true` (turbo-test#183).
fn vitest_runtime_config(entry: &Path) -> Option<String> {
    let dir = entry.parent()?;
    thread_local! {
        static RUNTIME_CONFIG_MEMO: RefCell<HashMap<PathBuf, Option<String>>> = RefCell::new(HashMap::new());
    }
    if let Some(v) = RUNTIME_CONFIG_MEMO.with(|m| m.borrow().get(dir).cloned()) {
        return v;
    }
    let v = vitest_runtime_config_uncached(dir);
    RUNTIME_CONFIG_MEMO.with(|m| m.borrow_mut().insert(dir.to_path_buf(), v.clone()));
    v
}

fn vitest_runtime_config_uncached(start: &Path) -> Option<String> {
    let env = std::env::var("TURBO_ZERO_DELAY_TIMERS").ok();
    static WARNED: std::sync::Once = std::sync::Once::new();
    if let Some(w) = zero_delay_env_warning(env.as_deref()) {
        WARNED.call_once(|| eprintln!("{w}"));
    }
    runtime_config_for(start, env.as_deref())
}

/// The runtime literal for files under `start`: the nearest config's settings, and the
/// zeroDelayTimers env switch (what the CLI flag sets) with or without a config file.
fn runtime_config_for(start: &Path, env: Option<&str>) -> Option<String> {
    // A jest config is read only when no vitest config governs the project (README): the
    // nearest jest config is kept while the walk goes on, and any vitest config above wins.
    let mut jest: Option<String> = None;
    let mut dir = Some(start);
    while let Some(d) = dir {
        for cfg in VITEST_CONFIG_FILES {
            if let Ok(s) = std::fs::read_to_string(d.join(cfg)) {
                return runtime_config_literal_with(&s, env);
            }
        }
        if jest.is_none() {
            jest = jest_config_text(d);
        }
        dir = d.parent();
    }
    match jest {
        Some(s) => jest_runtime_config_literal(&s, env),
        None => runtime_config_literal_with("", env),
    }
}

/// A jest config's runtime literal (turbo-test#186): its `fakeTimers` block as `jestFakeTimers`
/// (enableGlobally and jest.useFakeTimers' defaults, runtime.js), and the zeroDelayTimers env
/// switch as for a vitest config.
fn jest_runtime_config_literal(s: &str, env: Option<&str>) -> Option<String> {
    let base = runtime_config_literal_with("", env);
    let Some(ft) = config_object_literal(s, "fakeTimers") else {
        return base;
    };
    Some(match base {
        Some(b) => format!("{{ jestFakeTimers: {ft}, {}", &b[2..]),
        None => format!("{{ jestFakeTimers: {ft} }}"),
    })
}

/// The warning for a `TURBO_ZERO_DELAY_TIMERS` value that is not `0|1|true|false` (ignored).
fn zero_delay_env_warning(env: Option<&str>) -> Option<String> {
    match env {
        None | Some("0" | "1" | "true" | "false") => None,
        Some(v) => Some(format!(
            "turbo-test: ignoring TURBO_ZERO_DELAY_TIMERS={v:?} (expected 0, 1, true or false)"
        )),
    }
}

/// [`vitest_runtime_config`]'s literal from a config's text (`None` when it sets none of them).
#[cfg(test)]
fn runtime_config_literal(s: &str) -> Option<String> {
    runtime_config_literal_with(s, None)
}

fn runtime_config_literal_with(s: &str, env: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(ft) = config_object_literal(s, "fakeTimers") {
        parts.push(format!("fakeTimers: {ft}"));
    }
    for key in [
        "clearMocks",
        "mockReset",
        "restoreMocks",
        "unstubEnvs",
        "unstubGlobals",
    ] {
        if config_is_true(s, key) {
            parts.push(format!("{key}: true"));
        }
    }
    if let Some(on) = zero_delay_timers_from(env, s) {
        parts.push(format!("zeroDelayTimers: {on}"));
    }
    (!parts.is_empty()).then(|| format!("{{ {} }}", parts.join(", ")))
}

/// The zeroDelayTimers switch when something sets it (the runtime's default is on):
/// `TURBO_ZERO_DELAY_TIMERS=0|1` (what `--[no-]zero-delay-timers` sets) wins over the config's
/// `test.zeroDelayTimers: true|false`.
fn zero_delay_timers_from(env: Option<&str>, s: &str) -> Option<bool> {
    match env {
        Some("0" | "false") => return Some(false),
        Some("1" | "true") => return Some(true),
        _ => {}
    }
    let start = find_config_key(s, "zeroDelayTimers")?;
    let (_, v) = s[start..].split_once(':')?;
    let v = v.trim_start();
    v.starts_with("true")
        .then_some(true)
        .or_else(|| v.starts_with("false").then_some(false))
}

/// A DOM switch: Chrome's behaviour or jsdom's (the default, vitest parity), from the env
/// variable `var` (what the CLI flag sets) or the nearest config's `test.<key>`, memoized by
/// the test file's directory. `TURBO_CHROME_MEDIA` / `chromeMedia`: turbo-dom `MediaMode`
/// (a live `matchMedia`, `@media` against the 1024x768 viewport). `TURBO_CSS_TRANSITIONS` /
/// `cssTransitions`: CSS transitions, `@keyframes` animations and Web Animations.
fn dom_switch(entry: &Path, var: &'static str, key: &str) -> bool {
    let dir = entry.parent().unwrap_or(entry);
    thread_local! {
        static SWITCH_MEMO: RefCell<HashMap<(&'static str, PathBuf), bool>> = RefCell::new(HashMap::new());
    }
    let memo_key = (var, dir.to_path_buf());
    if let Some(v) = SWITCH_MEMO.with(|m| m.borrow().get(&memo_key).copied()) {
        return v;
    }
    let env = std::env::var(var).ok();
    warn_switch_env_once(var, env.as_deref());
    let v = switch_for(dir, env.as_deref(), key);
    SWITCH_MEMO.with(|m| m.borrow_mut().insert(memo_key, v));
    v
}

/// [`dom_switch`] for files under `start`: the nearest config's `test.<key>`, or the env
/// switch with or without a config file.
fn switch_for(start: &Path, env: Option<&str>, key: &str) -> bool {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for cfg in VITEST_CONFIG_FILES {
            if let Ok(s) = std::fs::read_to_string(d.join(cfg)) {
                return switch_from(env, &s, key);
            }
        }
        dir = d.parent();
    }
    switch_from(env, "", key)
}

/// The env switch `0|1|true|false` (what the flag sets) wins over the config's
/// `test.<key>: true|false`; neither set means jsdom's behaviour.
fn switch_from(env: Option<&str>, s: &str, key: &str) -> bool {
    match env {
        Some("0" | "false") => false,
        Some("1" | "true") => true,
        _ => config_is_true(s, key),
    }
}

/// Print [`switch_env_warning`] the first time `var` has a bad value; whether it printed.
fn warn_switch_env_once(var: &'static str, env: Option<&str>) -> bool {
    static WARNED: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let Some(w) = switch_env_warning(var, env) else {
        return false;
    };
    let mut warned = WARNED.lock().unwrap();
    if warned.contains(&var) {
        return false;
    }
    warned.push(var);
    eprintln!("{w}");
    true
}

/// The warning for an env switch value that is not `0|1|true|false` (ignored).
fn switch_env_warning(var: &str, env: Option<&str>) -> Option<String> {
    match env {
        None | Some("0" | "1" | "true" | "false") => None,
        Some(v) => Some(format!(
            "turbo-test: ignoring {var}={v:?} (expected 0, 1, true or false)"
        )),
    }
}

/// Whether a config key's value is `true`.
fn config_is_true(s: &str, key: &str) -> bool {
    find_config_key(s, key).is_some_and(|start| {
        s[start..]
            .split_once(':')
            .is_some_and(|(_, v)| v.trim_start().starts_with("true"))
    })
}

/// The `{ ... }` value of a config key, braces included (`None` when the key is absent or its
/// value is not an object literal).
fn config_object_literal(s: &str, key: &str) -> Option<String> {
    let start = find_config_key(s, key)?;
    let colon = start + s[start..].find(':')?;
    let open = colon + 1 + s[colon + 1..].find(|c: char| !c.is_whitespace())?;
    if s.as_bytes()[open] != b'{' {
        return None;
    }
    let (inner, _) = crate::launcher::balanced_block_end(s, open)?;
    Some(format!("{{{inner}}}"))
}

/// The quoted strings of a config key's `[...]` list: empty when the key is absent or its value
/// is not a literal list (a variable's list is not another key's, turbo-test#191).
fn config_string_list(s: &str, key: &str) -> Vec<String> {
    let list = || {
        let start = find_config_key(s, key)?;
        let colon = start + s[start..].find(':')?;
        let rest = s[colon + 1..].trim_start().strip_prefix('[')?;
        Some(&rest[..rest.find(']')?])
    };
    list().map_or_else(Vec::new, |inner| {
        inner
            .split(',')
            .map(|part| {
                part.trim()
                    .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                    .to_string()
            })
            .filter(|q| !q.is_empty())
            .collect()
    })
}

/// Add the config's snapshot serializers (`__ttLoadSnapshotSerializers` in runtime.js): a file
/// whose module has no default object, or whose default cannot print, fails with vitest's error.
fn load_snapshot_serializers(scope: &mut v8::PinScope, files: &[PathBuf]) -> Result<(), String> {
    let list: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
    let js = format!(
        "globalThis.__ttLoadSnapshotSerializers({})",
        serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
    );
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    if let Some(code) = v8::String::new(tc, &js) {
        if let Some(script) = v8::Script::compile(tc, code, None) {
            script.run(tc);
        }
    }
    caught_message(tc, "snapshot serializers failed").map_or(Ok(()), Err)
}

/// Jest config parity: read `setupFiles` + `setupFilesAfterEnv` from a jest config in `dir`
/// (`jest.config.{js,cjs,mjs,ts,json}` or a `"jest"` block in package.json), resolving Jest's
/// `<rootDir>` token (rootDir defaults to the config dir). Returns None if no jest config here
/// (so the caller keeps walking up); Some(possibly-empty) once a jest config is found.
fn jest_setup_files(dir: &Path) -> Option<Vec<PathBuf>> {
    let s = jest_config_text(dir)?;
    jest_setup_files_from(dir, &s)
}

/// The text of the jest config in `dir`: `jest.config.{js,cjs,mjs,ts,json}`, else the `"jest"`
/// block of package.json; None when `dir` has neither.
fn jest_config_text(dir: &Path) -> Option<String> {
    let names = [
        "jest.config.js",
        "jest.config.cjs",
        "jest.config.mjs",
        "jest.config.ts",
        "jest.config.json",
    ];
    let mut text: Option<String> = None;
    for n in &names {
        if let Ok(s) = std::fs::read_to_string(dir.join(n)) {
            text = Some(s);
            break;
        }
    }
    // package.json "jest" block (only if no standalone config file)
    if text.is_none() {
        text = std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|pkg| package_json_jest_block(&pkg));
    }
    text
}

fn jest_setup_files_from(dir: &Path, s: &str) -> Option<Vec<PathBuf>> {
    // rootDir (default = config dir). Jest paths in setupFiles use `<rootDir>`.
    let root_dir = config_string_value(s, "rootDir")
        .map(|r| normalize_path(&dir.join(&r)))
        .unwrap_or_else(|| dir.to_path_buf());
    let mut out = Vec::new();
    for key in ["setupFiles", "setupFilesAfterEnv"] {
        if let Some(start) = find_config_key(s, key) {
            let tail = &s[start..];
            if let Some(lb) = tail.find('[') {
                if let Some(rb) = tail[lb..].find(']') {
                    for part in tail[lb + 1..lb + rb].split(',') {
                        let q = part
                            .trim()
                            .trim_matches(|c| c == '\'' || c == '"' || c == '`');
                        if q.is_empty() {
                            continue;
                        }
                        // substitute <rootDir>, then resolve relative to root_dir (or config dir)
                        let replaced = q.replace("<rootDir>", &root_dir.to_string_lossy());
                        let candidate = if Path::new(&replaced).is_absolute() {
                            normalize_path(Path::new(&replaced))
                        } else {
                            normalize_path(&dir.join(&replaced))
                        };
                        if candidate.is_file() {
                            out.push(candidate);
                        } else if let Some(abs) = resolve_spec(q, dir) {
                            out.push(abs);
                        }
                    }
                }
            }
        }
    }
    Some(out)
}

/// First string value for a config key (`key: 'value'` / `"value"`). Used for jest `rootDir`.
fn config_string_value(s: &str, key: &str) -> Option<String> {
    let start = find_config_key(s, key)?;
    let tail = s[start..].trim_start();
    // skip the key name + colon
    let after_colon = tail.find(':').map(|c| &tail[c + 1..])?.trim_start();
    let q = after_colon.chars().next()?;
    if q == '\'' || q == '"' || q == '`' {
        let rest = &after_colon[1..];
        let end = rest.find(q)?;
        Some(rest[..end].to_string())
    } else {
        None
    }
}

/// The top-level `"jest"` object of a package.json, re-serialized as JSON text (Jest reads its
/// config from that key only; a `jest` key nested elsewhere, e.g. under `scripts`, is not it).
/// None when package.json is not valid JSON or has no `"jest"` object.
fn package_json_jest_block(pkg: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(pkg).ok()?;
    let jest = v.get("jest")?;
    jest.is_object().then(|| jest.to_string())
}

/// Normalize a path lexically (resolve `..`/`.`) without touching the filesystem.
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Run a setup module in the current context. Module-runner: like the test file itself (vitest
/// runs setup files through the same module runner), so its vi.mock()s are hoisted above its own
/// imports: the mock prepass registers them, then the setup module loads as CJS with its requires
/// below its vi.mock calls. Otherwise (no esbuild CJS transform, e.g. top-level await): bundled,
/// with mock specifiers externalized, then any vi.mock() calls it made are drained (resolved
/// relative to the setup file's dir) — they reach the test files, not the setup's own imports.
fn run_setup_file(
    scope: &mut v8::PinScope,
    file: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
) -> Result<(), String> {
    // A setup file that throws fails every test file that uses it, with its error (vitest).
    let failed = |e: String| Err(format!("setup file {}: {e}", file.display()));
    if mr_enabled() && esbuild_transform_cjs(file, false).is_some() {
        run_entry_mocks(scope, file).or_else(failed)?;
        // the setup's vi.hoisted() calls re-hit the prepass cache by index; afterwards the cache
        // is cleared so the test file's own vi.hoisted() calls never read the setup's values
        run_js(scope, "globalThis.__hoistedIdx = 0;");
        let err = {
            let tc = std::pin::pin!(v8::TryCatch::new(scope));
            let tc = &mut tc.init();
            let loaded = load_cjs(tc, file, false).is_some();
            if loaded {
                tc.perform_microtask_checkpoint();
            }
            caught_message(tc, "load failed")
                .or_else(|| (!loaded).then(|| "load failed".to_string()))
        };
        run_js(
            scope,
            "globalThis.__hoistedCache = []; globalThis.__hoistedIdx = 0;",
        );
        return err.map_or(Ok(()), failed);
    }
    // vitest hoists the setup's vi.mock calls above its imports here too (turbo-test#194): the
    // mock prepass registers them before the bundle's externalized imports load (every loader
    // path: turbo-test#196)
    run_entry_mocks(scope, file).or_else(failed)?;
    run_js(
        scope,
        "globalThis.__hoistedCache = []; globalThis.__hoistedIdx = 0;",
    );
    // reset the pending-mock queue before this setup file runs
    {
        let global = scope.get_current_context().global(scope);
        if let Some(key) = v8::String::new(scope, "__pendingMocks") {
            let empty = v8::Array::new(scope, 0);
            global.set(scope, key.into(), empty.into());
        }
    }
    let target = esbuild_bundle_full(file, externals, rewrite_map, None)
        .unwrap_or_else(|| file.to_path_buf());
    let err = match load_entry_graph(scope, &target) {
        Err(e) => Some(e),
        Ok(module) => {
            let tc = std::pin::pin!(v8::TryCatch::new(scope));
            let tc = &mut tc.init();
            if module.instantiate_module(tc, resolve_callback) == Some(true) {
                let ev = module.evaluate(tc);
                tc.perform_microtask_checkpoint();
                let caught = caught_message(tc, "unknown");
                let exc = match module.get_status() {
                    v8::ModuleStatus::Errored => Some(module.get_exception()),
                    _ => tc.exception(),
                };
                note_load_error(tc, exc);
                evaluate_error(caught, ev.is_some(), module.get_status(), || {
                    module.get_exception().to_rust_string_lossy(tc)
                })
            } else {
                Some(instantiate_error(caught_message(tc, "unknown")))
            }
        }
    };
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, file.parent().unwrap_or(Path::new(".")), true);
    err.map_or(Ok(()), failed)
}

/// The ESM-graph mock prepass: run `first` (the hoisted declarations + the vi.mock calls), else
/// `fallback` (the calls alone) when it throws or does not compile. A prepass that cannot
/// compile at all fails the file with the SyntaxError: its mocks would otherwise fall through
/// silently to the real modules (turbo-test#79 item 1). A factory that throws registers its
/// error stand-in in runtime.js.
fn run_prepass(scope: &mut v8::PinScope, first: &str, fallback: &str) -> Result<(), String> {
    if run_js_ok(scope, first) {
        return Ok(());
    }
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    let code = v8::String::new(tc, fallback).ok_or("prepass too long")?;
    match v8::Script::compile(tc, code, None) {
        Some(script) => {
            let _ = script.run(tc);
            Ok(())
        }
        None => Err(format!(
            "{} (in the file's vi.mock factories)",
            caught_message(tc, "SyntaxError").unwrap_or_default()
        )),
    }
}

/// Run `js` as a script; whether it completed without throwing (the exception is cleared).
fn run_js_ok(scope: &mut v8::PinScope, js: &str) -> bool {
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    v8::String::new(tc, js)
        .and_then(|code| v8::Script::compile(tc, code, None))
        .and_then(|s| s.run(tc))
        .is_some()
}

/// Run a short framework snippet in the current context.
fn run_js(scope: &mut v8::PinScope, js: &str) {
    if let Some(code) = v8::String::new(scope, js) {
        if let Some(s) = v8::Script::compile(scope, code, None) {
            s.run(scope);
        }
    }
}

/// Extract the `vi.hoisted(...)` statements (full statements) so they can be replayed in the
/// mock prepass — mock factories commonly close over hoisted vars. A declared call
/// (`const|let|var <binding> = [await] vi.hoisted(...)`) is taken from its keyword; a bare call
/// (`[await] vi.hoisted(...)`, also hoisted by vitest) from the call itself. A statement ends at
/// the call's closing paren, plus a `;` that follows it.
fn extract_hoisted(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut i = 0;
    while let Some(p) = src.get(i..).and_then(|s| s.find("vi.hoisted(")) {
        let at = i + p;
        i = at + "vi.hoisted(".len();
        let head = src.get(..at).unwrap_or("");
        // `await` directly before the call belongs to it
        let lead = head.trim_end();
        let call_start = match lead.strip_suffix("await") {
            Some(pre) if !pre.chars().next_back().is_some_and(ident) => pre.len(),
            _ => at,
        };
        let before_call = src.get(..call_start).unwrap_or("").trim_end();
        // declared: the nearest `const `/`let `/`var ` keyword whose text up to the call is just
        // `<binding> =` (no statement boundary in between); otherwise a bare call
        let start = ["const ", "let ", "var "]
            .iter()
            .filter_map(|kw| head.rfind(kw))
            .max()
            .filter(|&kw| {
                let between = src.get(kw..call_start).unwrap_or("");
                !head[..kw].chars().next_back().is_some_and(ident)
                    && before_call.ends_with('=')
                    && !before_call.ends_with("==")
                    && !between.contains(';')
                    && !between.contains("vi.hoisted(")
            })
            .unwrap_or(call_start);
        // balance parens from vi.hoisted( to its close, then include a following ';'
        let mut depth = 1i32;
        let mut k = i;
        while k < bytes.len() && depth > 0 {
            match bytes[k] as char {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        let mut end = k;
        while end < bytes.len() && (bytes[end] as char).is_whitespace() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] as char == ';' {
            k = end + 1;
        }
        out.push(src.get(start..k).unwrap_or("").to_string());
        i = k.max(at + 1);
    }
    out
}

/// Register the ENTRY file's own vi.mock()s before it instantiates. The mock calls are
/// emitted into a generated pre-pass module, bundled with esbuild (so JSX/import factories
/// transform correctly), executed (factories run → __pendingMocks), then drained relative to
/// the entry dir. This is how hoisted entry mocks work with bundling.
fn run_entry_mocks(scope: &mut v8::PinScope, entry: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(entry).unwrap_or_default();
    let dir = entry.parent().unwrap_or(Path::new("."));
    // The dir the entry's runtime vi.mock / vi.doMock calls resolve from when the module has no
    // __ttDir (ESM graph): its hoisted calls' prepass keys (runtime.js __ttPrepassMocks) and
    // immediate registration of a vi.doMock. Set for every entry (also one without vi.mock).
    let entry_dir = js_string_literal(&dir.to_string_lossy());
    run_js(scope, &format!("globalThis.__ttEntryDir = {entry_dir};"));
    let mocks = extract_mocks(&src);
    if mocks.is_empty() {
        return Ok(());
    }
    let Some(root) = project_root(entry) else {
        return run_entry_mocks_untransformed(scope, entry, &src, &entry_dir, &mocks);
    };
    let p = write_entry_prepass(&root, &src, &mocks);
    {
        let global = scope.get_current_context().global(scope);
        if let Some(key) = v8::String::new(scope, "__pendingMocks") {
            let empty = v8::Array::new(scope, 0);
            global.set(scope, key.into(), empty.into());
        }
    }
    if mr_enabled() {
        run_entry_mocks_mr(scope, entry, &src, &p);
        return Ok(());
    }
    run_entry_mocks_bundled(scope, entry, &p);
    Ok(())
}

/// The `vi.mock(...)` / `jest.mock(...)` replay calls of the entry's prepass (one per mock).
fn prepass_mock_calls(src: &str, mocks: &[(String, String)]) -> String {
    // a jest.mock stays one: its factory is its own default (turbo-test#137)
    let jest = extract_mock_calls(src, ["jest.mock(", "jest.mock("]);
    mocks
        .iter()
        .map(|(spec, factory)| {
            let s = js_string_literal(spec);
            let api = if jest.iter().any(|(j, f)| j == spec && f == factory) {
                "jest"
            } else {
                "vi"
            };
            if factory.is_empty() {
                format!("{api}.mock({s});\n")
            } else {
                format!("{api}.mock({s}, {factory});\n")
            }
        })
        .collect()
}

/// Entry mocks without an esbuild project (ESM graph).
fn run_entry_mocks_untransformed(
    scope: &mut v8::PinScope,
    entry: &Path,
    src: &str,
    entry_dir: &str,
    mocks: &[(String, String)],
) -> Result<(), String> {
    let dir = entry.parent().unwrap_or(Path::new("."));
    // No esbuild project (ESM graph): the runtime runs this file's prepass like the esbuild
    // one, just untransformed — one script that replays the file's vi.hoisted declarations
    // and then its vi.mock calls, so factories close over the hoisted values (vitest); async
    // factories are awaited by the resolve + drain below. __ttDir is the entry dir only
    // while it runs (automocks and importOriginal resolve from it). A throwing factory
    // registers runtime.js's error stand-in, as the module's own call would. If the
    // replayed declarations fail, the mocks are registered without them.
    let hoisted = extract_hoisted(src).join("\n");
    let calls = prepass_mock_calls(src, mocks);
    run_js(
        scope,
        &format!(
            "globalThis.__ttPrevDir = globalThis.__ttDir; globalThis.__ttDir = {entry_dir}; \
             globalThis.__hoistedCache = []; globalThis.__hoistedIdx = 0;"
        ),
    );
    // the factories see the file's top-level bindings through its scope (mock_scope.rs)
    let bindings = factory_bindings(entry, src, false);
    // the factories (and hoisted declarations) get the file's own TS / JSX transform, as
    // vitest transforms the whole file; in the script it becomes, `import.meta` is the
    // entry's and `require` (the JSX runtime's) resolves from it (turbo-test#79 item 1)
    let scoped = |code: String| scope_prepass_code(entry, entry_dir, &bindings, code);
    let prepass = run_prepass(
        scope,
        &scoped(format!("(() => {{\n{hoisted}\n{calls}}})();")),
        &scoped(format!("(() => {{\n{calls}}})();")),
    );
    run_js(scope, "globalThis.__ttDir = globalThis.__ttPrevDir;");
    prepass?;
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, dir, false);
    // These registrations are this file's prepass: the module's own vi.mock call keeps them
    // instead of re-running the factory.
    for (spec, _) in mocks {
        let Some(abs) = mock_key(spec, dir) else {
            continue;
        };
        if REGISTRY.with(|r| r.borrow().mocks.contains_key(&abs)) {
            let key = js_string_literal(&abs.to_string_lossy());
            run_js(scope, &format!("__ttPrepassMocks.add({key});"));
        }
    }
    Ok(())
}

/// Transform one untransformed-prepass script like the entry: TS/JSX, `import.meta`, the file's
/// top-level bindings scope, and the entry's `require`.
fn scope_prepass_code(
    entry: &Path,
    entry_dir: &str,
    bindings: &[crate::mock_scope::Binding],
    code: String,
) -> String {
    let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("js");
    let require = format!("globalThis.__mkRequire({entry_dir}, true)");
    // (runtime.js __ttImportMeta: url / filename / dirname / env, turbo-test#123)
    let meta = format!(
        "globalThis.__ttImportMeta({})",
        js_string_literal(&entry.to_string_lossy()),
    );
    let code = if crate::transform::needs_transform(entry) {
        crate::transform::transform(Path::new(&format!("prepass.{ext}")), &code).unwrap_or(code)
    } else {
        code
    };
    let code = crate::transform::replace_import_meta(&code, &meta);
    let code = if bindings.is_empty() {
        code
    } else {
        crate::mock_scope::with_scope(&code, bindings, entry)
    };
    // the transform's own `require("react/jsx-runtime")` (a file without imports)
    format!("((require) => {{\n{code}\n}})({require});")
}

/// Write the entry's esbuild mock prepass module (hashed, inside node_modules) and return it.
fn write_entry_prepass(root: &Path, src: &str, mocks: &[(String, String)]) -> PathBuf {
    // React is imported here too: mock factories commonly reference the test's module-scope
    // `React` (e.g. `React.useEffect` inside a returned component) which isn't in this prepass.
    let mut content = String::from("import { vi } from 'vitest';\nimport React from 'react';\n");
    // vi.hoisted() declarations that the mock factories reference must exist in the prepass.
    for decl in extract_hoisted(src) {
        content.push_str(&decl);
        content.push('\n');
    }
    for (spec, factory) in mocks {
        content.push_str(&format!("vi.mock({:?}, {});\n", spec, factory));
    }
    // Route module-`let`s the factories close over through globalThis.__ms so they share the
    // SAME binding as the entry module (which esbuild_transform_cjs rewrites identically).
    let shared = shared_mock_lets(src);
    if !shared.is_empty() {
        content = rewrite_shared_lets(&content, &shared);
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut h);
    // Write the prepass INSIDE node_modules (gitignored) so esbuild resolves bare imports
    // injected by the JSX transform (react/jsx-runtime) from the file's directory upward.
    let pre_dir = root.join("node_modules/.turbo-test-prepass");
    let _ = std::fs::create_dir_all(&pre_dir);
    let p = pre_dir.join(format!("prepass-{:016x}.tsx", h.finish()));
    // atomic write (temp + rename) so parallel jobs writing the same-hash prepass file don't
    // race a reader into a partial file ("entry load failed" flakiness under -j).
    if std::fs::read_to_string(&p).ok().as_deref() != Some(content.as_str()) {
        write_atomic(&p, &content);
    }
    p
}

/// Module-runner entry prepass: the factories run IN THE CURRENT realm.
fn run_entry_mocks_mr(scope: &mut v8::PinScope, entry: &Path, src: &str, p: &Path) {
    let dir = entry.parent().unwrap_or(Path::new("."));
    // Module-runner: run the mock factories IN THE CURRENT realm (CJS transform + wrapper)
    // so their require('react')/jsx-runtime resolve to the SAME shared react the test uses
    // — React-component mocks then render correctly (no dual-react). Runs before the entry.
    // Guard the whole prepass: an async factory's `await import('<self>')` resolves during the
    // microtask checkpoint below, re-entering load_cjs for the real module. Its nested drain
    // would clear the pending-mock queue and register the still-pending factory promise as an
    // empty mock (so the real module wins and named imports never rebind to the spies). The
    // flag makes load_cjs leave the queue alone until the outer drain runs here.
    let prev_resolving = RESOLVING_MOCKS.with(|f| f.replace(true));
    set_mock_prepass(scope, true);
    if let Some(raw) = esbuild_transform_cjs(p, false) {
        // the factories see the file's top-level bindings through its scope (mock_scope.rs)
        let bindings = factory_bindings(entry, src, true);
        let raw = if bindings.is_empty() {
            raw
        } else {
            crate::mock_scope::with_scope(&raw, &bindings, entry)
        };
        run_prepass_wrapper(scope, entry, dir, &raw);
    }
    set_mock_prepass(scope, false);
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, entry.parent().unwrap_or(Path::new(".")), false);
    RESOLVING_MOCKS.with(|f| f.set(prev_resolving));
}

/// Compile + call the CJS-wrapped module-runner prepass with the entry's require / dir.
fn run_prepass_wrapper(scope: &mut v8::PinScope, entry: &Path, dir: &Path, raw: &str) {
    let wrapped =
        format!("(function (exports, module, require, __filename, __dirname) {{\n{raw}\n}})");
    if let Some(code) = v8::String::new(scope, &wrapped) {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        if let Some(script) = v8::Script::compile(tc, code, None) {
            if let Some(wf) = script
                .run(tc)
                .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
            {
                let exports_obj = v8::Object::new(tc);
                let module_obj = v8::Object::new(tc);
                if let Some(ek) = v8::String::new(tc, "exports") {
                    module_obj.set(tc, ek.into(), exports_obj.into());
                }
                let dir_str = v8::String::new(tc, &dir.to_string_lossy()).unwrap();
                let g = tc.get_current_context().global(tc);
                // expose the entry dir for vi.importActual('...') used inside factories
                if let Some(k) = v8::String::new(tc, "__ttDir") {
                    g.set(tc, k.into(), dir_str.into());
                }
                // fresh vi.hoisted cache for this file; the prepass fills it (the entry
                // reuses it by index so `const x = vi.hoisted(...)` is one shared object).
                if let Some(code) = v8::String::new(
                    tc,
                    "globalThis.__hoistedCache = []; globalThis.__hoistedIdx = 0;",
                ) {
                    if let Some(s) = v8::Script::compile(tc, code, None) {
                        s.run(tc);
                    }
                }
                if let Some(req) = v8::String::new(tc, "__mkRequire")
                    .and_then(|k| g.get(tc, k.into()))
                    .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
                {
                    let undef = v8::undefined(tc).into();
                    if let Some(require) = req.call(
                        tc,
                        undef,
                        &[dir_str.into(), v8::Boolean::new(tc, true).into()],
                    ) {
                        let fname = v8::String::new(tc, &entry.to_string_lossy()).unwrap();
                        wf.call(
                            tc,
                            undef,
                            &[
                                exports_obj.into(),
                                module_obj.into(),
                                require,
                                fname.into(),
                                dir_str.into(),
                            ],
                        );
                        tc.perform_microtask_checkpoint();
                    }
                }
            }
        }
    }
}

/// Non-module-runner entry prepass: bundle it with esbuild and evaluate it as an ES module.
fn run_entry_mocks_bundled(scope: &mut v8::PinScope, entry: &Path, p: &Path) {
    let dir = entry.parent().unwrap_or(Path::new("."));
    let bundled = esbuild_bundle(p, &[]).unwrap_or_else(|| p.to_path_buf());
    set_mock_prepass(scope, true);
    // the factories' relative specifiers (a factory-less mock's automock importActual, and
    // vi.importActual / importOriginal inside factories) resolve from the entry dir, as on the
    // module-runner branch above; the bundle itself lives in the cache dir
    let global = scope.get_current_context().global(scope);
    if let (Some(k), Some(v)) = (
        v8::String::new(scope, "__ttDir"),
        v8::String::new(scope, &dir.to_string_lossy()),
    ) {
        global.set(scope, k.into(), v.into());
    }
    {
        // (a load error the prepass throws is the entry's own, reported when the entry loads)
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        if let Some(module) = load_graph(tc, &bundled) {
            if module.instantiate_module(tc, resolve_callback) == Some(true) {
                let _ = module.evaluate(tc);
                tc.perform_microtask_checkpoint();
            }
        }
    }
    set_mock_prepass(scope, false);
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, entry.parent().unwrap_or(Path::new(".")), false);
}

/// Mark the entry's mock prepass for runtime.js. The prepass runs the factories outside the
/// module's scope (no imports or module-level bindings), so a factory that throws THERE is not a
/// factory error: it is not registered, and the module's own `vi.mock` call (which re-runs the
/// factory in scope) decides. Registering it would break the module's "needed" requires that load
/// the mocked module before that call (e.g. for a later `vi.mocked(x)`).
fn set_mock_prepass(scope: &mut v8::PinScope, on: bool) {
    let global = scope.get_current_context().global(scope);
    if let Some(k) = v8::String::new(scope, "__ttMockPrepass") {
        let v = v8::Boolean::new(scope, on);
        global.set(scope, k.into(), v.into());
    }
}

/// Set up the DOM environment in the current context: bind the all-Rust DOM (rtdom) natively so
/// document/window exist before the test evaluates. (The legacy JS turbo-dom bootstrap —
/// installGlobals + the `.node` parser — was removed in v0.3.0; rtdom is the only DOM now.)
fn setup_dom(scope: &mut v8::PinScope, entry: &Path) {
    turbo_dom_browser_env::browser_env::install(scope);
    // jsdom's media by default (vitest parity); Chrome's with --chrome-media,
    // TURBO_CHROME_MEDIA=1 or test.chromeMedia: true
    turbo_dom_browser_env::browser_env::set_media_mode(
        scope,
        if dom_switch(entry, "TURBO_CHROME_MEDIA", "chromeMedia") {
            turbo_dom_browser_env::browser_env::MediaMode::Chrome
        } else {
            turbo_dom_browser_env::browser_env::MediaMode::Jsdom
        },
    );
    // jsdom's: no CSS transitions / animations and no Web Animations by default (vitest
    // parity); Chrome's with --css-transitions, TURBO_CSS_TRANSITIONS=1 or
    // test.cssTransitions: true. Independent of the media switch.
    // turbo-dom 0.8 gates transitions and @keyframes animations separately (turbo-dom#313);
    // turbo-test's one switch drives both, so the default stays jsdom's (neither runs).
    let css_motion = dom_switch(entry, "TURBO_CSS_TRANSITIONS", "cssTransitions");
    turbo_dom_browser_env::browser_env::set_css_transitions(scope, css_motion);
    turbo_dom_browser_env::browser_env::set_css_animations(scope, css_motion);
    // the embedder's window members: open / alert / print / postMessage … (turbo-test#208)
    run_js(
        scope,
        "globalThis.__ttInstallWindowMembers && globalThis.__ttInstallWindowMembers();",
    );
    // opt-in jsdom quirks (TURBO_JSDOM_QUIRKS=1, turbo-test#210): under environment jsdom,
    // getComputedStyle reports jsdom's cascaded declared values; off (the default) it stays Chrome's
    let quirks = std::env::var("TURBO_JSDOM_QUIRKS").is_ok_and(|v| v == "1" || v == "true");
    run_js(scope, &format!("globalThis.__ttJsdomQuirks = {quirks};"));
}

/// Transform a TS file to **ESM** JS using the PROJECT'S OWN TypeScript (`ts.transpileModule`),
/// lowering decorators + `emitDecoratorMetadata` with exact ts-jest parity. Unlike esbuild (no
/// metadata) and oxc 0.134 (emits `Object` for type-alias / nullable types), tsc resolves local
/// type aliases (`type Percentage = number` -> `Number`) so NestJS/Mongoose decorators get the
/// right `design:type`. Module syntax is kept as ESM so the caller's esbuild ESM->CJS pass gives
/// the same `var import_X = require(...)` shape the rest of the pipeline (vi.mock hoisting) needs.
/// Imports used only in type positions are first moved onto a namespace import (TypeScript's own
/// remedy for TS1272), so on the ESM graph an interface's metadata reads `ns.Iface` (undefined →
/// `Object`) instead of naming an export that doesn't exist.
/// Returns None if the project ships no `typescript` (caller falls back to oxc).
fn tsc_transform_esm(file: &Path, root: &Path) -> Option<String> {
    let src = read_app_source(file)?;
    let ts_dir = root.join("node_modules/typescript");
    if !ts_dir.join("package.json").is_file() {
        return None;
    }
    // Small node program: transpile with the project's TS (no checker — same isolatedModules
    // semantics ts-jest uses), keeping ESM module syntax + inlined helpers + decorator metadata.
    const SCRIPT: &str = r#"
const ts = require(process.argv[1]);
const fs = require('fs');
const file = process.argv[2];
const src = fs.readFileSync(0, 'utf8'); // the module's source (import.meta.glob expanded)
const isTsx = file.endsWith('.tsx');
// isolatedModules + emitDecoratorMetadata: a value import of a name used only as a type (an
// interface) is kept for its `design:type` reference and then fails ESM linking (no such export).
// TypeScript's own remedy (TS1272) is a namespace import: `ns.Shape` reads as undefined -> Object.
// So import bindings referenced only in type positions are moved to a namespace import and their
// type references qualified with it. Names that are shadowed or used as values are left alone.
function nsTypeOnlyImports(text) {
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, isTsx ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const binds = new Map();
  for (const st of sf.statements) {
    const ic = ts.isImportDeclaration(st) && st.importClause;
    if (!ic || ic.isTypeOnly) continue;
    const add = (local, imported, el) => binds.set(local.text, { st, el, imported, refs: [], ok: true });
    if (ic.name) add(ic.name, 'default', ic);
    if (ic.namedBindings && ts.isNamedImports(ic.namedBindings)) {
      for (const el of ic.namedBindings.elements) {
        const imp = el.propertyName || el.name;
        if (!el.isTypeOnly && ts.isIdentifier(imp)) add(el.name, imp.text, el);
      }
    }
  }
  if (!binds.size) return text;
  const declKinds = [ts.isVariableDeclaration, ts.isParameter, ts.isFunctionDeclaration, ts.isFunctionExpression,
    ts.isClassDeclaration, ts.isClassExpression, ts.isInterfaceDeclaration, ts.isTypeAliasDeclaration,
    ts.isEnumDeclaration, ts.isModuleDeclaration, ts.isTypeParameterDeclaration, ts.isBindingElement,
    ts.isImportEqualsDeclaration, ts.isNamespaceImport, ts.isImportSpecifier, ts.isImportClause];
  const typeUse = (id) => {
    for (let n = id, q = id.parent; q; n = q, q = q.parent) {
      if (ts.isTypeQueryNode(q) || ts.isComputedPropertyName(q)) return false;
      if (ts.isExpressionWithTypeArguments(q)) {
        const h = q.parent;
        return ts.isHeritageClause(h) && (h.token === ts.SyntaxKind.ImplementsKeyword || ts.isInterfaceDeclaration(h.parent));
      }
      if (ts.isTypeNode(q)) return true;
    }
    return false;
  };
  const visit = (node) => {
    const b = ts.isIdentifier(node) && binds.get(node.text);
    if (b) {
      const p = node.parent;
      if (p === b.el) { /* the import binding itself */ }
      else if (declKinds.some((k) => k(p)) && p.name === node) b.ok = false; // shadowed
      else if (ts.isExportSpecifier(p)) b.ok = false; // re-exported: needs the binding
      else if ((ts.isPropertyAccessExpression(p) && p.name === node) || (ts.isQualifiedName(p) && p.right === node)
        || (p.propertyName === node) || (p.name === node && !ts.isShorthandPropertyAssignment(p))
        || ts.isLabeledStatement(p) || ts.isBreakOrContinueStatement(p)) { /* a member name, not a reference */ }
      else if (typeUse(node)) b.refs.push(node);
      else b.ok = false; // a value reference
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  const edits = [];
  const perDecl = new Map();
  for (const b of binds.values()) {
    if (!b.ok || !b.refs.length) continue;
    if (!perDecl.has(b.st)) perDecl.set(b.st, { ns: '__tt_ns' + perDecl.size, moved: new Set() });
    const d = perDecl.get(b.st);
    d.moved.add(b.el);
    for (const r of b.refs) edits.push([r.getStart(sf), r.end, d.ns + '.' + b.imported]);
  }
  for (const [st, d] of perDecl) {
    const ic = st.importClause;
    const nb = ic.namedBindings;
    const keepDefault = ic.name && !d.moved.has(ic);
    const keepNamed = nb && ts.isNamedImports(nb) ? nb.elements.filter((e) => !d.moved.has(e)) : [];
    const from = st.moduleSpecifier.getText(sf);
    const parts = [];
    if (keepDefault) parts.push(ic.name.text);
    if (nb && ts.isNamespaceImport(nb)) parts.push(nb.getText(sf));
    if (keepNamed.length) parts.push('{ ' + keepNamed.map((e) => e.getText(sf)).join(', ') + ' }');
    const clause = parts.length ? 'import ' + parts.join(', ') + ' from ' + from + '; ' : '';
    edits.push([st.getStart(sf), st.end, clause + 'import * as ' + d.ns + ' from ' + from + ';']);
  }
  edits.sort((a, b) => b[0] - a[0]);
  let out = text;
  for (const [s, e, r] of edits) out = out.slice(0, s) + r + out.slice(e);
  return out;
}
let input = src;
try { input = nsTypeOnlyImports(src); } catch (e) {} // e.g. a typescript without the parser API
const out = ts.transpileModule(input, {
  fileName: file,
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2021,
    experimentalDecorators: true,
    emitDecoratorMetadata: true,
    esModuleInterop: true,
    importHelpers: false,
    isolatedModules: true,
    jsx: isTsx ? ts.JsxEmit.ReactJSX : undefined,
    useDefineForClassFields: false,
  },
});
process.stdout.write(out.outputText);
"#;
    let mut child = std::process::Command::new("node")
        .current_dir(root)
        .args([
            "-e",
            SCRIPT,
            &ts_dir.to_string_lossy(),
            &file.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
        child.stdin.take()?.write_all(src.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let code = String::from_utf8(out.stdout).ok()?;
    if code.trim().is_empty() {
        return None;
    }
    Some(code)
}

/// Convert JS module syntax ESM->CJS with esbuild (stdin), no bundling/resolution — imports stay
/// as `require(...)` and exports become `module.exports`/`exports.x`. Used as the second pass for
/// the oxc decorator-metadata output (which oxc leaves as ESM). The input is already plain JS
/// (TS/decorators lowered), so this is a pure format conversion.
fn esbuild_format_cjs(root: &Path, js: &str) -> Option<String> {
    use std::io::Write;
    let esbuild = esbuild_bin(root);
    let mut child = std::process::Command::new(&esbuild)
        .current_dir(root)
        .args([
            "--format=cjs",
            "--platform=node",
            "--loader=js",
            "--log-level=silent",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(js.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(postprocess_mr_cjs(String::from_utf8(out.stdout).ok()?))
}

/// Does the nearest tsconfig enable `emitDecoratorMetadata`? Cached per tsconfig path (the file
/// is read once per project). Used to decide whether decorator files need the oxc metadata path.
fn decorator_metadata_enabled(file: &Path) -> bool {
    thread_local! {
        static CACHE: RefCell<HashMap<PathBuf, bool>> = RefCell::new(HashMap::new());
    }
    let Some(tc) = nearest_tsconfig(file.parent().unwrap_or(Path::new("."))) else {
        return false;
    };
    if let Some(v) = CACHE.with(|c| c.borrow().get(&tc).copied()) {
        return v;
    }
    // Strip `//` line comments before scanning so a commented mention doesn't false-positive.
    let on = std::fs::read_to_string(&tc)
        .map(|s| {
            let stripped: String = s
                .lines()
                .map(|l| l.split("//").next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n");
            let norm = stripped.replace(char::is_whitespace, "");
            norm.contains("\"emitDecoratorMetadata\":true")
        })
        .unwrap_or(false);
    CACHE.with(|c| c.borrow_mut().insert(tc, on));
    on
}

/// Cheap syntactic check: does the source use a decorator? A decorator appears either at the
/// start of a line (`@Injectable()`, `  @Prop()`) or inline as a parameter decorator (`(@Inject()`).
fn file_has_decorator(src: &str) -> bool {
    if src.contains("(@") {
        return true;
    }
    src.lines().any(|l| {
        let t = l.trim_start();
        let mut ch = t.chars();
        ch.next() == Some('@')
            && ch
                .next()
                .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                .unwrap_or(false)
    })
}

/// Native (oxc) counterpart of `esbuild_transform_cjs` for app files (P2a). Emits the same
/// esbuild-shaped, runner-friendly CJS via `esm_cjs::emit` (no esbuild subprocess, no
/// `postprocess_mr_cjs` — native output already has configurable getters + identity `__toESM`),
/// then applies the SAME vi.mock hoisting + shared-let rewrite as the esbuild path so mocks behave
/// identically. Content-cached under a distinct key. Returns `None` on any unhandled form → caller
/// falls back to esbuild.
fn native_transform_cjs(file: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let raw = with_binder(file, read_app_source(file)?, true);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    file.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .hash(&mut h);
    // v4: await_do_mocks_on_import; v5: import.meta (turbo-test#123); v6: `__toESM` is
    // runtime.js's `__ttToESM` (turbo-test#122)
    // v7: ES-module namespaces without `__esModule`, `export *` of the namespace (turbo-test#133)
    "native-cjs-v7".hash(&mut h);
    // Legacy-decorator lowering happens inside esm_cjs::emit when the project enables
    // experimentalDecorators/emitDecoratorMetadata AND this file uses a decorator. Without it the
    // native emitter leaves `@Decorator` syntax intact (→ "Unexpected token 'export'") or lowers
    // it with 2022-standard semantics (→ "Cannot access 'X' before initialization" TDZ). Key the
    // cache on this decision so a config/file change can't serve stale (mis-lowered) output.
    let legacy_decorators = decorator_metadata_enabled(file) && file_has_decorator(&raw);
    legacy_decorators.hash(&mut h);
    // coverage output carries an inline source map → key it separately so the no-map and with-map
    // forms never collide in the cache.
    crate::coverage::enabled().hash(&mut h);
    let cache = cache_dir().join(format!("ntv-{:016x}.cjs", h.finish()));
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let emitted = crate::esm_cjs::emit_with(file, &raw, legacy_decorators)?;
    let mut code = hoist_mock_setup(&await_do_mocks_on_import(emitted));
    let shared = shared_mock_lets(&raw);
    if !shared.is_empty() {
        code = rewrite_shared_lets(&code, &shared);
    }
    write_atomic(&cache, &code);
    Some(code)
}

/// `native_transform_cjs` for the test file under test locations (location.rs): the same code,
/// built uncached with the emitter's source map, whose lines follow the mock hoisting and the
/// shared-`let` prelude so the map stays aligned with the code that runs.
fn native_transform_cjs_located(file: &Path) -> Option<String> {
    let (code, segs, raw) = native_transform_cjs_mapped(file)?;
    crate::location::set_map(file, segs, &raw);
    Some(code)
}

/// `native_transform_cjs` with the map of its lines back to the source (and that source): the
/// test file's for test locations, another app module's while failure frames are printed.
/// The map of an app module's `native_transform_cjs` lines back to its source, for a failure
/// frame in it (`location::set_module_map_lazy`): the mapped transform emits the same code.
fn native_module_segments(file: &Path) -> Option<crate::location::Segments> {
    native_transform_cjs_mapped(file).map(|(_, segs, _)| segs)
}

#[cfg(test)]
thread_local! {
    /// Uncached mapped transforms on this thread (the gate in `runner_tests_a`).
    static MAPPED_TRANSFORMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn native_transform_cjs_mapped(file: &Path) -> Option<(String, crate::location::Segments, String)> {
    #[cfg(test)]
    MAPPED_TRANSFORMS.with(|n| n.set(n.get() + 1));
    let raw = with_binder(file, read_app_source(file)?, true);
    let legacy_decorators = decorator_metadata_enabled(file) && file_has_decorator(&raw);
    let emitted = crate::esm_cjs::emit_mapped(file, &raw, legacy_decorators)?;
    let map = crate::coverage::inline_segments(&emitted)?;
    let (mut code, mut origins) = hoist_mock_setup_tracked(&await_do_mocks_on_import(emitted));
    let shared = shared_mock_lets(&raw);
    if !shared.is_empty() {
        code = rewrite_shared_lets(&code, &shared);
        origins.insert(0, None);
    }
    let segs = origins
        .iter()
        .map(|o| o.and_then(|i| map.get(i).cloned()).unwrap_or_default())
        .collect();
    Some((code, segs, raw))
}

/// Native (oxc) counterpart of `esbuild_bundle_dep_cjs` for node_modules files (P2b). Instead of
/// bundling a package's relative files into one module (esbuild `--bundle --packages=external`),
/// transform the SINGLE file ESM→CJS via `esm_cjs::emit` and let the loader resolve its relative +
/// bare `require`s on demand — bare imports stay `require("react")` etc., so singletons resolve to
/// ONE instance via the require cache exactly like `--packages=external`. CJS/plain files pass
/// through unchanged (emit's no-module-syntax path). No vi.mock hoisting (deps don't mock). Cached.
/// Returns `None` on an unhandled form → caller falls back to the esbuild bundle.
fn native_dep_cjs(file: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let raw = std::fs::read_to_string(file).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    file.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .hash(&mut h);
    // v4: import.meta (turbo-test#123); v5: `__toESM` is runtime.js's `__ttToESM` (turbo-test#122)
    // v6: package interop `__ttToESMDep`, Node's for a package Node runs as ES modules
    // (turbo-test#133)
    // v7: package ES modules marked "pkg" (turbo-test#159)
    // v8: each module's closure takes the bundle's helpers as parameters (turbo-test#230)
    "native-dep-v8-bundle".hash(&mut h);
    // the bundle inlines the package's whole relative graph, not just this file: two installs
    // with the same entry file (user-event 14.6.1 / 14.6.8) differ inside, so the key names this
    // install (its path) and its package.json (turbo-test#213)
    file.hash(&mut h);
    if let Some(manifest) = file
        .ancestors()
        .skip(1)
        .map(|d| d.join("package.json"))
        .find(|p| p.is_file())
    {
        std::fs::read(&manifest).unwrap_or_default().hash(&mut h);
    }
    let node = is_esm_module(file);
    node.hash(&mut h);
    let cache = cache_dir().join(format!("ntvdep-{:016x}.cjs", h.finish()));
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    // Bundle the package's relative graph with lazy __commonJS init wrappers (circular-safe),
    // bare imports left external (shared via require cache). None → caller falls back to esbuild.
    let code = dep_interop(crate::bundler::bundle(file)?, node);
    write_atomic(&cache, &code);
    Some(code)
}

/// Module-runner mode: transform a single APP module to CJS (no bundle) so imports become
/// live `require(...).name` property access (mutable → spyOn/vi.mock work), and post-process
/// esbuild's export getters to be CONFIGURABLE (so spyOn can redefine them). Cached.
fn esbuild_transform_cjs(file: &Path, prefer_metadata: bool) -> Option<String> {
    let root = project_root(file)?;
    let esbuild = esbuild_bin(&root);
    // the test / setup file's factory-scope binder (turbo-test#205, #222), as native_transform_cjs
    let raw = with_binder(file, read_app_source(file)?, true);
    let use_meta = prefer_metadata && decorator_metadata_enabled(file) && file_has_decorator(&raw);
    let cov = crate::coverage::enabled();
    let cache = mr_cjs_cache_path(file, &raw, use_meta, cov);
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    if use_meta {
        if let Some(code) = esbuild_transform_cjs_meta(file, &root, &raw, &cache) {
            return Some(code);
        }
        // fall through to the plain esbuild transform below (metadata-less, but loads)
    }
    let tsconfig = find_tsconfig_up_to(file, &root);
    let code = esbuild_cjs_run(file, &root, &esbuild, &raw, &cache, cov, tsconfig)?;
    let code = postprocess_mr_cjs(code);
    // esbuild sorts the exports; vitest's namespace lists them in source order (turbo-test#141)
    let order = crate::transform::transform(file, &raw)
        .ok()
        .and_then(|js| crate::esm_cjs::export_order(&js));
    let code = match order {
        Some(order) => source_order_exports(code, &order),
        None => code,
    };
    let code = finish_mr_cjs(code, &raw);
    write_atomic(&cache, &code);
    Some(code)
}

/// import.meta fill, do-mock await, vi.mock hoisting and shared mock lets on the module-runner CJS.
fn finish_mr_cjs(code: String, raw: &str) -> String {
    let mut code = await_do_mocks_on_import(fill_import_meta(code, None));
    // vitest-style vi.mock hoisting: move consumer requires below the vi.mock setup so the mock
    // is registered (immediately, in this module's scope) before the mocked module loads.
    code = hoist_mock_setup(&code);
    // Module-`let`s that this file's vi.mock() factory closes over (writes from inside the mock,
    // read by the test) must be SHARED between the mock-prepass scope and this module's scope.
    // vitest shares them via one module scope; we can't (the factory runs in the prepass), so
    // route those lets through a shared global. Same rewrite is applied to the prepass factory.
    let shared = shared_mock_lets(raw);
    if !shared.is_empty() {
        code = rewrite_shared_lets(&code, &shared);
    }
    code
}

/// Cache path of `esbuild_transform_cjs`'s output for this source / transform flavour.
fn mr_cjs_cache_path(file: &Path, raw: &str, use_meta: bool, cov: bool) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    file.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .hash(&mut h);
    // v2: `import.meta` is filled in (fill_import_meta) instead of esbuild's empty object;
    // v3: import() waits for a pending async vi.doMock (await_do_mocks_on_import);
    // v4: import.meta.url is the percent-encoded file URL (turbo-test#87);
    // v5: import.meta is runtime.js __ttImportMeta (env, one object per module; turbo-test#123)
    // v6: `__toESM` is runtime.js's `__ttToESM`, `__toCommonJS` marks the module (turbo-test#122)
    // v7: `export *` re-exports the importer's namespace (turbo-test#133); exports in source
    // order (turbo-test#141)
    // v8: __reExport's __toESMStar calls globalThis.__ttToESM (turbo-test#165)
    "mr-cjs-v8".hash(&mut h);
    // esbuild can't emit decorator metadata. By default we DON'T pay that cost — only when a
    // decorator file actually threw at load (Mongoose @Prop / Sequelize @Column read design:type
    // at class-definition) does the loader retry with `prefer_metadata`, routing it through the
    // project's TypeScript (ts.transpileModule, exact ts-jest parity) so the metadata is emitted.
    // Retry-on-failure keeps the common case on fast esbuild and never regresses files that load
    // fine without metadata. Keyed separately so the two transforms never collide in cache.
    use_meta.hash(&mut h);
    // Coverage emits an inline source map (needed to remap V8 byte ranges → original lines), so
    // the output differs — key it separately. Only perturb the key when coverage is ON, so the
    // normal (no-map) cache keys are byte-identical to before → zero cache churn on default runs.
    if cov {
        "cov-inline-map".hash(&mut h);
    }
    cache_dir().join(format!("mr-{:016x}.cjs", h.finish()))
}

/// The decorator-metadata transform of `esbuild_transform_cjs` (tsc, else oxc, then esbuild CJS).
fn esbuild_transform_cjs_meta(file: &Path, root: &Path, raw: &str, cache: &Path) -> Option<String> {
    // Primary: the project's own TypeScript (ts.transpileModule) → exact ts-jest parity for
    // emitDecoratorMetadata (resolves local type aliases, nullable types, etc.). Fallback:
    // oxc lowers decorators+metadata (ESM) then esbuild converts ESM->CJS — used when the
    // project ships no `typescript`. oxc can panic on some inputs, so it's caught; either way
    // we fall through to the plain esbuild transform (metadata-less but loads) on failure.
    // Get ESM-with-metadata from tsc (preferred) or oxc (fallback), then a single esbuild
    // ESM->CJS pass so every path yields the same esbuild-shaped CJS the loader/hoisting want.
    let meta_esm = tsc_transform_esm(file, root).or_else(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::transform::transform_decorators_with_metadata(file, raw)
        }))
        .ok()
        .and_then(|r| r.ok())
    });
    let cjs = meta_esm.and_then(|esm| esbuild_format_cjs(root, &esm))?;
    let code = finish_mr_cjs(postprocess_mr_cjs(cjs), raw);
    write_atomic(cache, &code);
    Some(code)
}

/// Run esbuild's per-file CJS transform for `esbuild_transform_cjs`; its raw stdout.
fn esbuild_cjs_run(
    file: &Path,
    root: &Path,
    esbuild: &Path,
    raw: &str,
    cache: &Path,
    cov: bool,
    tsconfig: Option<PathBuf>,
) -> Option<String> {
    // A module whose source is not the file's (import.meta.glob expanded, turbo-test#130; the
    // binder appended) is transformed from a copy beside the cache entry, not from stdin: esbuild
    // rejects `--tsconfig` in stdin (transform) mode, so every such module failed the transform
    // in a project with a tsconfig.json (turbo-test#223). Same extension, same loader; the
    // appended binder is on a line of its own, so line numbers (coverage's map) do not move.
    let expanded = std::fs::read_to_string(file).ok().as_deref() != Some(raw);
    let input = if expanded {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("js");
        // per process + call, so a parallel job transforming the same source never removes it
        let seq = ATOMIC_SEQ.fetch_add(1, Ordering::Relaxed);
        let copy = cache.with_extension(format!("src{}-{seq}.{ext}", std::process::id()));
        std::fs::write(&copy, raw).ok()?;
        copy
    } else {
        file.to_path_buf()
    };
    let mut cmd = std::process::Command::new(esbuild);
    cmd.current_dir(root);
    cmd.arg(&input);
    if expanded {
        // as the stdin loader was: a .js / unknown-extension module may hold JSX
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("js");
        if !matches!(ext, "ts" | "mts" | "cts" | "tsx" | "jsx") {
            cmd.arg(format!("--loader:.{ext}=jsx"));
        }
    }
    cmd.args([
        "--format=cjs",
        "--platform=browser",
        "--jsx=automatic",
        "--log-level=silent",
        "--define:process.env.NODE_ENV=globalThis.process.env.NODE_ENV",
        "--supported:dynamic-import=false",
    ]);
    // Under --coverage: emit an inline source map so coverage.rs can map V8 byte ranges back to
    // the original .ts line. (No --sourcefile: that's stdin-only; with a file arg esbuild names
    // the source itself, which is fine — we attribute to the known file path directly.)
    if cov {
        cmd.args(["--sourcemap=inline", "--sources-content=false"]);
    }
    if let Some(tc) = &tsconfig {
        cmd.arg(format!("--tsconfig={}", tc.display()));
    }
    let out = cmd.stderr(std::process::Stdio::null()).output();
    if expanded {
        let _ = std::fs::remove_file(&input);
    }
    let out = out.ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// esbuild's `--format=cjs` turns `import.meta` into an empty object (`const import_meta = {};`,
/// one per source module), so `import.meta.url` read as undefined. Give it the module's own
/// import.meta instead, as vitest/Node do: runtime.js `__ttImportMeta(<path>)` (`url` — the
/// percent-encoded file URL the ESM-graph callback gives too — `filename`, `dirname` and vitest's
/// `env`, turbo-test#123). A per-file transform's module is the CJS wrapper's `__filename` at run
/// time (the transform cache is content-keyed, so the same source in two places shares one
/// output). In a dep bundle (`bundle_root`: esbuild's cwd) each module's code follows esbuild's
/// `// <path relative to the cwd>` comment, which names it. Replaced on the same line, so the
/// inline source map (coverage) keeps its line mapping.
fn fill_import_meta(code: String, bundle_root: Option<&Path>) -> String {
    if !code.contains("import_meta") {
        return code;
    }
    let mut out = String::with_capacity(code.len() + 96);
    let mut module: Option<String> = None;
    for line in code.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        if let (Some(root), Some(rel)) = (bundle_root, body.strip_prefix("// ")) {
            let abs = root.join(rel);
            if abs.is_file() {
                module = Some(js_string_literal(&abs.to_string_lossy()));
            }
        }
        let decl = body
            .strip_prefix("const ")
            .or_else(|| body.strip_prefix("var "))
            .and_then(|r| r.strip_suffix(" = {};"))
            .filter(|n| {
                n.strip_prefix("import_meta")
                    .is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()))
            });
        match decl {
            Some(name) => {
                let kw = &body[..body.find(' ').unwrap_or(0)];
                let path = module.as_deref().unwrap_or("__filename");
                out.push_str(&format!("{kw} {name} = globalThis.__ttImportMeta({path});"));
                out.push_str(&line[body.len()..]);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// vitest-style vi.mock hoisting on esbuild CJS output: esbuild emits all `require(...)` at the
/// top, then the body (module decls + vi.mock + tests). A vi.mock must be registered before the
/// mocked module loads, so we move the requires NOT referenced by the mock-setup region (the
/// "consumers" — incl. the modules being mocked) to AFTER that region. The setup's own decls run
/// in-module-scope, so the factory shares the test's bindings (push, mockUseAuthInfo, etc.).
fn hoist_mock_setup(code: &str) -> String {
    hoist_mock_setup_tracked(code).0
}

/// `hoist_mock_setup`, also returning for each output line the input line it came from (`None` for
/// a line it wrote: a mocked require's re-bind).
fn hoist_mock_setup_tracked(code: &str) -> (String, Vec<Option<usize>>) {
    let unchanged = || {
        (
            code.to_string(),
            (0..code.split('\n').count()).map(Some).collect(),
        )
    };
    if !code.contains(".mock(") {
        return unchanged();
    }
    let lines: Vec<&str> = code.lines().collect();
    let is_req = |l: &str| l.starts_with("var import_") && l.contains("require(");
    // contiguous require block
    let Some(req_start) = lines.iter().position(|l| is_req(l)) else {
        return unchanged();
    };
    let mut req_end = req_start;
    while req_end + 1 < lines.len() && is_req(lines[req_end + 1]) {
        req_end += 1;
    }
    // end of the last top-level `.mock(` statement (balance parens from the call to its close + ;)
    // (`code` contains ".mock(" and that needle has no newline, so some line always matches; the
    // 0 default would hit the `<= req_end` early return below anyway.)
    let last_mock = lines
        .iter()
        .rposition(|l| l.contains(".mock("))
        .unwrap_or(0);
    if last_mock <= req_end {
        return unchanged();
    }
    let region_end = mock_stmt_end(&lines, last_mock);
    let setup = &lines[req_end + 1..=region_end];
    // Split the setup region into: pre-mock decls (factory-referenced vars declared BEFORE the
    // first vi.mock — must stay before, the factory closes over them), the vi.mock statements
    // themselves, and post-mock decls (e.g. `const x = vi.mocked(import_y.z)` interleaved with /
    // after the mocks — must run AFTER the requires are re-bound to the mocks). esbuild emits
    // factory-dep decls before the first mock and capture decls after, so split at the 1st mock.
    let first_mock_rel = setup.iter().position(|l| l.contains(".mock(")).unwrap_or(0);
    let pre_setup: Vec<&str> = setup[..first_mock_rel].to_vec();
    // walk the tail, separating full `.mock(...)` statements from interleaved decls
    let (mock_stmts, post_decls) = split_mock_stmts(&setup[first_mock_rel..]);
    // A require stays BEFORE the vi.mocks if its value is needed at LOAD time — by a mock factory
    // or by top-level code (e.g. `class X extends React.Component`). A require used only inside
    // test/hook callbacks runs later, so it can be a consumer that loads AFTER the mocks (this is
    // what makes `vi.mock` placed below the tests still intercept). We test references against the
    // setup region with test-callback bodies stripped, plus the factory bodies explicitly.
    let setup_text_full = setup.join("\n");
    let setup_text = format!(
        "{}\n{}",
        strip_test_blocks(&setup_text_full),
        mock_stmts.join("\n")
    );
    let (needed, consumers) = partition_requires(&lines[req_start..=req_end], &setup_text);
    let rebinds = mocked_require_rebinds(setup, &lines[req_start..=req_end]);
    let mut out = Vec::with_capacity(lines.len() + rebinds.len());
    out.extend_from_slice(&lines[..req_start]);
    out.extend(needed);
    // factory-dep decls, then the vi.mock calls, then re-bind mocked requires, then the capture
    // decls (vi.mocked) which now read the mock, then consumer requires, then the body.
    out.extend(pre_setup);
    out.extend(mock_stmts);
    for r in &rebinds {
        out.push(r.as_str());
    }
    out.extend(post_decls);
    out.extend(consumers);
    if region_end + 1 < lines.len() {
        out.extend_from_slice(&lines[region_end + 1..]);
    }
    // an output line that is one of the input's slices came from that line
    let index: HashMap<usize, usize> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (l.as_ptr() as usize, i))
        .collect();
    let origins = out
        .iter()
        .map(|l| index.get(&(l.as_ptr() as usize)).copied())
        .collect();
    (out.join("\n"), origins)
}

/// Track `(` / `)` nesting over `line`: `started` once a `(` is seen.
fn track_parens(line: &str, depth: &mut i32, started: &mut bool) {
    for ch in line.chars() {
        match ch {
            '(' => {
                *depth += 1;
                *started = true;
            }
            ')' => *depth -= 1,
            _ => {}
        }
    }
}

/// The line on which the statement opening at `from` closes its parens (`from` if none open).
fn mock_stmt_end(lines: &[&str], from: usize) -> usize {
    let mut depth = 0i32;
    let mut started = false;
    for (i, l) in lines.iter().enumerate().skip(from) {
        track_parens(l, &mut depth, &mut started);
        if started && depth <= 0 {
            return i;
        }
    }
    from
}

/// Split the mock-setup tail into its full `.mock(...)` statements and the interleaved decls.
fn split_mock_stmts<'a>(tail: &[&'a str]) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut mock_stmts: Vec<&str> = Vec::new();
    let mut post_decls: Vec<&str> = Vec::new();
    let mut k = 0usize;
    while k < tail.len() {
        if tail[k].contains(".mock(") {
            let mut depth = 0i32;
            let mut started = false;
            let start = k;
            while k < tail.len() {
                track_parens(tail[k], &mut depth, &mut started);
                k += 1;
                if started && depth <= 0 {
                    break;
                }
            }
            mock_stmts.extend_from_slice(&tail[start..k]);
        } else {
            post_decls.push(tail[k]);
            k += 1;
        }
    }
    (mock_stmts, post_decls)
}

/// Requires whose binding the setup text references (needed before the mocks) vs consumers.
fn partition_requires<'a>(reqs: &[&'a str], setup_text: &str) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut needed = Vec::new();
    let mut consumers = Vec::new();
    for &l in reqs {
        // var import_NAME = ...
        let name = l.strip_prefix("var ").unwrap_or(l);
        let name: String = name
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !name.is_empty() && contains_word(setup_text, &name) {
            needed.push(l);
        } else {
            consumers.push(l);
        }
    }
    (needed, consumers)
}

/// Re-bind lines (`import_X = <rhs>;`) for the requires of specifiers mocked in `setup`.
fn mocked_require_rebinds(setup: &[&str], reqs: &[&str]) -> Vec<String> {
    // Specifiers that are vi.mock()'d in this file. A require of a mocked spec that we kept
    // BEFORE the vi.mock (because a later `const x = vi.mocked(import_y.z)` references it) loaded
    // the REAL module; after registration we must re-bind it to the mock so the test's reference
    // matches what consumers get. (Re-running `require` returns the registered mock from cache.)
    let mut mocked_specs: Vec<String> = Vec::new();
    for &l in setup {
        if let Some(p) = l.find(".mock(") {
            let after = &l[p + 6..];
            if let Some(spec) = extract_first_string_literal(after) {
                mocked_specs.push(spec);
            }
        }
    }
    let mut rebinds: Vec<String> = Vec::new();
    if !mocked_specs.is_empty() {
        for &l in reqs {
            if let Some(spec) = extract_require_spec(l) {
                if mocked_specs.iter().any(|m| m == &spec) {
                    // `var import_X = <rhs>;` -> `import_X = <rhs>;` (re-fetch -> mock)
                    rebinds.push(l.replacen("var ", "", 1));
                }
            }
        }
    }
    rebinds
}

/// The value of the first string literal (`"..."`, `'...'` or `` `...` ``) in `s`, escape
/// sequences decoded (an unterminated one runs to the end of `s`).
fn extract_first_string_literal(s: &str) -> Option<String> {
    let at = s.find(['"', '\'', '`'])?;
    Some(js_string_literal_at(&s[at..])?.0)
}

/// The JS string literal `s` starts with (`"..."`, `'...'`, or a template literal, read
/// verbatim apart from its escapes): its value with the escape sequences decoded (`\"`, `\\`,
/// `\n`, `\x41`, `\u0041`, `\u{1F600}`, line continuations; any other escaped character stands for
/// itself), and its length in bytes through the closing quote, None when unterminated. None
/// when `s` does not start with a quote.
fn js_string_literal_at(s: &str) -> Option<(String, Option<usize>)> {
    let quote = s.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut out = String::new();
    let mut i = 1;
    while let Some(c) = s[i..].chars().next() {
        i += c.len_utf8();
        if c == quote {
            return Some((out, Some(i)));
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = s[i..].chars().next() else {
            break;
        };
        i += e.len_utf8();
        // \xHH, \uHHHH, \u{H...}: the code point (a malformed one stands for its letter)
        let hex = |digits: &str| {
            let valid = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit());
            valid
                .then(|| u32::from_str_radix(digits, 16).ok())
                .flatten()
                .and_then(char::from_u32)
        };
        let escaped = match e {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'v' => Some('\u{b}'),
            '0' => Some('\0'),
            'x' => s.get(i..i + 2).and_then(hex).inspect(|_| i += 2),
            'u' if s[i..].starts_with('{') => s[i + 1..].find('}').and_then(|n| {
                let c = hex(&s[i + 1..i + 1 + n]);
                c.inspect(|_| i += n + 2)
            }),
            'u' => s.get(i..i + 4).and_then(hex).inspect(|_| i += 4),
            // a line continuation stands for nothing (\r\n is one line terminator)
            '\r' => {
                i += usize::from(s[i..].starts_with('\n'));
                None
            }
            '\n' | '\u{2028}' | '\u{2029}' => None,
            _ => Some(e),
        };
        match escaped {
            Some(c) => out.push(c),
            None if matches!(e, 'x' | 'u') => out.push(e),
            None => {}
        }
    }
    Some((out, None))
}

/// From `var import_X = require("SPEC");` or `... = __toESM(require("SPEC"), 1);` -> "SPEC".
fn extract_require_spec(line: &str) -> Option<String> {
    let p = line.find("require(")?;
    extract_first_string_literal(&line[p + 8..])
}

/// Module-level `let`/`var` names a vi.mock() factory in this source references (so they must be
/// shared between the factory's prepass scope and the module's own scope).
fn shared_mock_lets(src: &str) -> Vec<String> {
    let mocks = extract_mocks(src);
    if mocks.is_empty() {
        return Vec::new();
    }
    let factories: String = mocks
        .iter()
        .map(|(_, f)| f.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        let (kw, is_const) = if let Some(r) = t.strip_prefix("let ") {
            (Some(r), false)
        } else if let Some(r) = t.strip_prefix("var ") {
            (Some(r), false)
        } else if let Some(r) = t.strip_prefix("const ") {
            (Some(r), true)
        } else {
            (None, false)
        };
        if let Some(rest) = kw {
            if let Some(name) = shared_let_name(src, &factories, rest, is_const, &out) {
                out.push(name);
            }
        }
    }
    out
}

/// The declared name of a `let`/`var`/`const` (`rest` after the keyword) when the mock
/// factories share it with the module (and it isn't already in `seen`).
fn shared_let_name(
    src: &str,
    factories: &str,
    rest: &str,
    is_const: bool,
    seen: &[String],
) -> Option<String> {
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if name.is_empty() || seen.contains(&name) {
        return None;
    }
    let shareable = if is_const {
        const_name_shareable(&name, rest, factories)
    } else {
        let_name_shareable(&name, src, factories)
    };
    // Skip if the name is used in object-literal shorthand (`{ name }`) — word-replacing
    // it to `globalThis.__ms.name` there is invalid JS (would break the whole file).
    (shareable && !used_in_shorthand(src, &name)).then_some(name)
}

/// `const` can't be reassigned → only an ARRAY accumulator (`= []`) the factory
/// mutates/reads. Arrays (`.push`) are rarely used in object shorthand/keys, so
/// word-replacing them is safe; objects/values are NOT (`{ x }`, `x:` would break).
/// `rest` is the text after `const`. `len>=12`: only long unique names; short ones (or mock
/// fns used in shorthand/strings) risk corrupting other code.
fn const_name_shareable(name: &str, rest: &str, factories: &str) -> bool {
    let is_array =
        rest.contains("= []") || rest.contains("= [\n") || rest.trim_end().ends_with("= [");
    name.len() >= 12 && is_array && contains_word(factories, name)
}

/// `let`/`var`: share when the factory REFERENCES the binding and it's assigned somewhere —
/// either by the factory itself (the capture pattern) or by the test bodies (the factory
/// closes over a mutable `let` that beforeEach/it reassign). `assigns_word(src)` covers both
/// directions (src includes the factory); `contains_word(factories)` keeps it to bindings
/// the factory actually uses.
fn let_name_shareable(name: &str, src: &str, factories: &str) -> bool {
    name.len() >= 6 && contains_word(factories, name) && assigns_word(src, name)
}

/// True if `word` appears as object-literal shorthand (`{ word }`, `{ word, ...}`, `..., word }`):
/// bounded by `{`/`,` before and `}`/`,` after (whitespace allowed), so it can't be safely
/// rewritten to `globalThis.__ms.word`.
fn used_in_shorthand(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(p) = text[i..].find(word) {
        let at = i + p;
        i = at + 1;
        if !is_whole_word_at(b, at, word.len()) {
            continue;
        }
        let (left, right) = neighbours_past_ws(b, at, at + word.len());
        if is_shorthand_bracket(left, right) {
            return true;
        }
    }
    false
}

/// A word of length `len` at byte offset `at` in `b` is a whole identifier (not glued to any
/// surrounding identifier chars on either side).
fn is_whole_word_at(b: &[u8], at: usize, len: usize) -> bool {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    if at > 0 && ident(b[at - 1]) {
        return false;
    }
    let end = at + len;
    if end < b.len() && ident(b[end]) {
        return false;
    }
    true
}

/// Object-literal shorthand bracketing: `{ word }`, `{ word, …}`, `{…, word }`.
/// (`,word,` middle is ambiguous with an arg list, so left alone.)
fn is_shorthand_bracket(left: u8, right: u8) -> bool {
    (left == b'{' && (right == b'}' || right == b',')) || (left == b',' && right == b'}')
}

/// The bytes before `at` and from `end` after skipping whitespace (incl newlines); 0 at an edge.
fn neighbours_past_ws(b: &[u8], at: usize, end: usize) -> (u8, u8) {
    let ws = |c: u8| c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';
    let mut l = at;
    while l > 0 && ws(b[l - 1]) {
        l -= 1;
    }
    let mut r = end;
    while r < b.len() && ws(b[r]) {
        r += 1;
    }
    let left = if l > 0 { b[l - 1] } else { 0 };
    let right = if r < b.len() { b[r] } else { 0 };
    (left, right)
}

/// True if `text` assigns `word` (`word =`, not `==`/`===`/`<=`/`>=`/`!=`), as a whole word.
fn assigns_word(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(p) = text[i..].find(word) {
        let at = i + p;
        i = at + 1;
        if is_word_assignment(b, at, word.len()) {
            return true;
        }
    }
    false
}

/// A bare `word` of length `word_len` found at byte `at` in `b` is being assigned to: it is a
/// whole identifier and is followed by `[ \t]*=` where the `=` is not the `==`/`=>` punctuator.
fn is_word_assignment(b: &[u8], at: usize, word_len: usize) -> bool {
    if !is_whole_word_at(b, at, word_len) {
        return false;
    }
    let mut j = at + word_len;
    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    j < b.len() && b[j] == b'=' && !matches!(b.get(j + 1), Some(b'=') | Some(b'>'))
}

/// True if `word` appears in `text` bounded by non-identifier chars.
/// Remove the bodies of test/hook callbacks (`it(...)`, `test(...)`, `describe(...)`,
/// `beforeEach/All`, `afterEach/All`, and `.only/.skip/.each` variants) from `text`, leaving
/// only top-level code (imports, declarations, class defs, vi.mock calls). Used to decide which
/// requires must load BEFORE the vi.mocks: a require referenced only inside a test callback runs
/// later (post-mock), so it can be a consumer; one used at top level (e.g. `class X extends
/// React.Component`) must stay before. Paren-balanced, with string/template/comment skipping.
fn strip_test_blocks(text: &str) -> String {
    let b = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let kws = [
        "beforeEach",
        "beforeAll",
        "afterEach",
        "afterAll",
        "describe",
        "test",
        "it",
        "fit",
        "xit",
        "fdescribe",
        "xdescribe",
    ];
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < b.len() {
        // a comment or a string literal is copied as it is: a keyword in it is no call
        // (turbo-test#224)
        if let Some(end) = comment_or_string_end(b, i) {
            out.push_str(&text[i..end]);
            i = end;
            continue;
        }
        // at an identifier boundary, see if a test keyword starts here
        let boundary = i == 0 || !ident(b[i - 1]);
        let matched = if boundary {
            test_keyword_end(b, i, &kws)
        } else {
            None
        };
        if let Some(j) = matched.and_then(|j| test_call_paren(b, j)) {
            let k = balanced_call_end(b, j);
            // emit the keyword name (so a require used in the callee position stays visible)
            out.push_str(&text[i..j]);
            i = k;
            continue;
        }
        // default: copy this char (advance by full UTF-8 char to keep valid boundaries)
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// The end of the test keyword in `kws` that starts at `i` as a whole identifier, if any.
fn test_keyword_end(b: &[u8], i: usize, kws: &[&str]) -> Option<usize> {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    for kw in kws.iter() {
        let kb = kw.as_bytes();
        if b[i..].starts_with(kb) {
            // next char must end the identifier (so `item`/`description` don't match `it`/`describe`)
            let after = i + kb.len();
            if after >= b.len() || !ident(b[after]) {
                return Some(after);
            }
        }
    }
    None
}

/// From just past a test keyword, the index of its call's opening `(` (None when not a call).
fn test_call_paren(b: &[u8], mut j: usize) -> Option<usize> {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    // skip whitespace + chained `.only` / `.skip` / `.each(...)` / `.concurrent` etc.
    // up to the opening '(' of the call.
    // allow the esbuild `(0, import_vitest.it)(...)` form: skip whitespace, `.`, idents,
    // and a single closing `)` before the call's opening `(`.
    while j < b.len() {
        let c = b[j];
        if c == b'(' {
            return Some(j);
        }
        if c.is_ascii_whitespace() || c == b'.' || c == b')' || ident(c) {
            j += 1;
            continue;
        }
        break; // something else → not a call we recognize
    }
    None
}

/// Balance parens from the `(` at `j`, skipping strings/templates/comments: the span's end.
fn balanced_call_end(b: &[u8], j: usize) -> usize {
    let mut depth = 0i32;
    let mut k = j;
    while k < b.len() {
        match b[k] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    k += 1;
                    break;
                }
            }
            b'\'' | b'"' | b'`' => k = skip_string_body(b, k),
            b'/' if k + 1 < b.len() && b[k + 1] == b'/' => k = skip_line_comment(b, k),
            b'/' if k + 1 < b.len() && b[k + 1] == b'*' => k = skip_block_comment(b, k),
            _ => {}
        }
        k += 1;
    }
    k
}

/// `balanced_call_end` string-body pass: `b[k]` is the opening quote; walk to the first matching
/// unescaped quote (or the end of `b`) and return its index.
fn skip_string_body(b: &[u8], k: usize) -> usize {
    let q = b[k];
    let mut k = k + 1;
    while k < b.len() {
        if b[k] == b'\\' {
            k += 2;
            continue;
        } else if b[k] == q {
            break;
        }
        k += 1;
    }
    k
}

/// `balanced_call_end` line-comment pass: walk to the next `\n` (or the end of `b`).
fn skip_line_comment(b: &[u8], mut k: usize) -> usize {
    while k < b.len() && b[k] != b'\n' {
        k += 1;
    }
    k
}

/// `balanced_call_end` block-comment pass: walk past the closing `*/` (or to the end of `b`).
fn skip_block_comment(b: &[u8], mut k: usize) -> usize {
    k += 2;
    while k + 1 < b.len() && !(b[k] == b'*' && b[k + 1] == b'/') {
        k += 1;
    }
    k + 1
}

/// The end (exclusive) of the `//` / `/* */` comment or `'` / `"` / `` ` `` string literal that
/// starts at `i` in `b`, through the end of `b` when it is unterminated. None when none starts
/// there. A string's backslash escapes are skipped.
fn comment_or_string_end(b: &[u8], i: usize) -> Option<usize> {
    let find = |from: usize, pat: &[u8]| {
        b[from.min(b.len())..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map_or(b.len(), |p| from + p + pat.len())
    };
    match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some(find(i + 2, b"\n")),
        (b'/', Some(b'*')) => Some(find(i + 2, b"*/")),
        (q @ (b'\'' | b'"' | b'`'), _) => {
            let mut k = i + 1;
            while k < b.len() && b[k] != q {
                k += if b[k] == b'\\' { 2 } else { 1 };
            }
            Some((k + 1).min(b.len()))
        }
        _ => None,
    }
}

fn contains_word(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let w = word.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut i = 0;
    while let Some(p) = text[i..].find(word) {
        let at = i + p;
        let before_ok = at == 0 || !ident(b[at - 1]);
        let after = at + w.len();
        let after_ok = after >= b.len() || !ident(b[after]);
        if before_ok && after_ok {
            return true;
        }
        i = at + 1;
    }
    false
}

/// Replace every whole-word occurrence of `word` in `text` with `repl`.
fn replace_word(text: &str, word: &str, repl: &str) -> String {
    let b = text.as_bytes();
    let w = word.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        // Skip string-literal interiors: a shared-let name that also appears as a quoted string
        // (e.g. `searchParams.get(k => k === 'mockRole' ? mockRole : null)`) must NOT have the
        // STRING rewritten — only the identifier. Copy the whole literal verbatim, honoring `\`
        // escapes. (Template `${...}` expressions are skipped too — a missed interpolation would
        // surface as a visible ReferenceError, never the silent corruption of a rewritten string.)
        let c = b[i];
        if let Some(end) = comment_run_end(text, b, i, c) {
            out.push_str(&text[i..end]);
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            let end = string_literal_end(text, i);
            out.push_str(&text[i..end]);
            i = end;
            continue;
        }
        if text[i..].starts_with(word) && is_rewritable_word_at(b, i, w.len()) {
            out.push_str(repl);
            i += w.len();
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// If `i` points at a `//` or `/* … */` comment, return the byte just past it. `None` otherwise.
/// Comments are copied as they are: a quote or backtick in one opens no string, and a name in
/// one is not a reference (turbo-test#209).
fn comment_run_end(text: &str, b: &[u8], i: usize, c: u8) -> Option<usize> {
    if c != b'/' || i + 1 >= b.len() || (b[i + 1] != b'/' && b[i + 1] != b'*') {
        return None;
    }
    if b[i + 1] == b'/' {
        return Some(text[i..].find('\n').map_or(text.len(), |n| i + n));
    }
    Some(
        text[i + 2..]
            .find("*/")
            .map_or(text.len(), |n| i + 2 + n + 2),
    )
}

/// End (exclusive) of the string literal opening at `i` (honoring `\` escapes; end of text when
/// unterminated).
fn string_literal_end(text: &str, mut i: usize) -> usize {
    let b = text.as_bytes();
    let c = b[i];
    i += 1;
    while i < text.len() {
        let d = b[i];
        if d == b'\\' && i + 1 < text.len() {
            i += 2;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        i += ch.len_utf8();
        if d == c {
            break;
        }
    }
    i
}

/// Whether the word at `i` follows `get ` / `set ` (an accessor's name).
fn is_accessor_name_at(b: &[u8], i: usize) -> bool {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut j = i;
    while j > 0 && (b[j - 1] == b' ' || b[j - 1] == b'\t') {
        j -= 1;
    }
    j >= 3
        && (&b[j - 3..j] == b"get" || &b[j - 3..j] == b"set")
        && (j == 3 || !ident(b[j - 4]))
        && j < i // there was whitespace between the keyword and the name
}

/// Whether the word of length `wlen` at `i` is a variable reference `replace_word` rewrites.
fn is_rewritable_word_at(b: &[u8], i: usize, wlen: usize) -> bool {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    // not preceded by an identifier char, nor a property-access `.` (so `obj.word` stays
    // intact). A spread `...word` (the `.` is part of `...`) IS the variable — rewrite it.
    let prop_dot = i > 0 && b[i - 1] == b'.' && !(i >= 2 && b[i - 2] == b'.');
    // accessor name (`get word()` / `set word()`): the NAME is a property key, not a
    // variable read — rewriting it yields `get globalThis.__ms.word()` (a syntax error).
    // The getter BODY's `return word` is preceded by `return `, so it still rewrites.
    let accessor_name = is_accessor_name_at(b, i);
    let before_ok = i == 0 || (!ident(b[i - 1]) && !prop_dot && !accessor_name);
    let after = i + wlen;
    // skip object-literal keys (`word:`) — replacing them yields `globalThis.__ms.word:`.
    let after_ok = after >= b.len() || (!ident(b[after]) && b[after] != b':');
    before_ok && after_ok
}

/// Route shared mock lets through `globalThis.__ms` so the mock prepass and this module share
/// one binding. Rewrites refs + turns the `let/var` declaration into a global assignment.
fn rewrite_shared_lets(code: &str, names: &[String]) -> String {
    let mut out = code.to_string();
    for name in names {
        let g = format!("globalThis.__ms.{name}");
        out = replace_word(&out, name, &g);
        // fix the (now `let/var/const globalThis.__ms.X`) declaration back into a plain assignment
        out = out.replace(&format!("let {g}"), &g);
        out = out.replace(&format!("var {g}"), &g);
        out = out.replace(&format!("const {g}"), &g);
    }
    format!("globalThis.__ms = globalThis.__ms || {{}};\n{out}")
}

/// Post-process esbuild CJS output for the module runner:
/// - make export getters CONFIGURABLE so `spyOn` can redefine them (mutable live exports);
/// - replace `__toESM` (which COPIES the namespace) with runtime.js's `__ttToESM`, which returns
///   the SAME exports object (adding `default` for CJS interop), or a view of it (a package's
///   CommonJS as Node's namespace, an ES module without `__esModule`), so `import * as ns` +
///   `import {x}` share one object — `vi.spyOn(ns,'x')` is then seen by every importer (vitest
///   semantics) — and `__toCommonJS` with one that marks the module;
/// - `export * from` re-exports that namespace's names (turbo-test#133).
fn postprocess_mr_cjs(code: String) -> String {
    postprocess_esbuild_cjs(
        code,
        "var __toESM = (m) => globalThis.__ttToESM(m);",
        // not `__toESM`: esbuild emits no such helper in a module that only re-exports
        "globalThis.__ttToESM(m)",
    )
}

/// `postprocess_mr_cjs` for a package's bundle: its `__toESM` is runtime.js's `__ttToESMDep`
/// (esbuild passes Node's interop flag for a `.mjs` / `"type": "module"` file), its `export *`
/// re-exports Node's namespace names when the package is one Node runs as ES modules (`node`).
fn postprocess_mr_dep_cjs(code: String, node: bool) -> String {
    package_esm_mark(postprocess_esbuild_cjs(
        code,
        "var __toESM = (m, n) => globalThis.__ttToESMDep(m, n);",
        if node {
            "globalThis.__ttToESMDep(m, 1)"
        } else {
            "m"
        },
    ))
}

/// A package's ES modules are marked `"pkg"` (runtime.js lists their namespace's names sorted,
/// as Node's, which vitest imports a package through; turbo-test#159).
fn package_esm_mark(code: String) -> String {
    code.replacen(
        "Symbol.for(\"turbo-test.esm\"), { value: true })",
        "Symbol.for(\"turbo-test.esm\"), { value: \"pkg\" })",
        1,
    )
}

fn postprocess_esbuild_cjs(mut code: String, to_esm: &str, star: &str) -> String {
    code = code.replace(
        "var __defProp = Object.defineProperty;",
        "var __defProp = (o, k, d) => Object.defineProperty(o, k, Object.assign({ configurable: true }, d));",
    );
    code = code.replace("var __toESM = ", &format!("{to_esm} var __toESM_unused = "));
    // turbo-test's own ES modules are marked, so `__ttToESM` tells them from CommonJS
    code = code.replace(
        "var __toCommonJS = (mod) => __copyProps(__defProp({}, \"__esModule\", { value: true }), mod);",
        "var __toCommonJS = (mod) => __copyProps(__defProp(__defProp({}, \"__esModule\", { value: true }), Symbol.for(\"turbo-test.esm\"), { value: true }), mod);",
    );
    code.replace(
        "var __reExport = (target, mod, secondTarget) => (",
        &format!(
            "var __toESMStar = (m) => {star}; var __reExport = (target, mod, secondTarget) => (mod = __toESMStar(mod), "
        ),
    )
}

/// esbuild's export registration (`__export(x_exports, {\n  b: () => b,\n  a: () => a\n});`,
/// sorted by name) with its entries in `order` (the module's source order); names `order` does
/// not know keep their place after them.
fn source_order_exports(code: String, order: &[String]) -> String {
    let Some(start) = code
        .find("_exports, {\n")
        .map(|i| i + "_exports, {\n".len())
    else {
        return code;
    };
    if !code[..start]
        .lines()
        .last()
        .is_some_and(|l| l.starts_with("__export("))
    {
        return code;
    }
    let Some(len) = code[start..].find("\n});") else {
        return code;
    };
    let block = &code[start..start + len];
    let key = |line: &str| {
        let k = line.trim_start().split(": () =>").next().unwrap_or("");
        serde_json::from_str::<String>(k).unwrap_or_else(|_| k.to_string())
    };
    let rank = |line: &str| {
        let k = key(line);
        order.iter().position(|o| *o == k).unwrap_or(order.len())
    };
    let mut lines: Vec<&str> = block.split('\n').map(|l| l.trim_end_matches(',')).collect();
    lines.sort_by_key(|l| rank(l));
    format!(
        "{}{}{}",
        &code[..start],
        lines.join(",\n"),
        &code[start + len..]
    )
}

/// A package's native bundle (esm_cjs's preamble) with the dependency interop of
/// `postprocess_mr_dep_cjs`: Node's for a package Node runs as ES modules (`node`).
fn dep_interop(code: String, node: bool) -> String {
    let n = if node { "1" } else { "0" };
    package_esm_mark(code)
        .replacen(
            crate::esm_cjs::TO_ESM_HELPER,
            &format!("var __toESM = (mod) => globalThis.__ttToESMDep(mod, {n});\n"),
            1,
        )
        .replacen(
            crate::esm_cjs::STAR_HELPER,
            if node {
                "var __toESMStar = (mod) => globalThis.__ttToESMDep(mod, 1);\n"
            } else {
                "var __toESMStar = (mod) => mod;\n"
            },
            1,
        )
}

/// The module-runner's lowered `import()` (esbuild's and esm_cjs's
/// `Promise.resolve().then(() => __toESM(require(SPEC)))`) waits for a pending async vi.doMock
/// factory of its module, as vitest awaits it (turbo-test#42 item 6):
/// `__ttImportReady(__dirname, SPEC)` (runtime.js) takes the place of `Promise.resolve()` for a
/// literal SPEC or esm_cjs's already-evaluated `__tt_spec`. It is `Promise.resolve()` itself
/// while nothing is pending, so the timing is unchanged. `__dirname` is the CJS wrapper's.
fn await_do_mocks_on_import(code: String) -> String {
    const LOWERED: &str = "Promise.resolve().then(() => __toESM(require(";
    if !code.contains(LOWERED) {
        return code;
    }
    let mut out = String::with_capacity(code.len() + 64);
    let mut rest = code.as_str();
    while let Some(at) = rest.find(LOWERED) {
        out.push_str(&rest[..at]);
        rest = &rest[at + LOWERED.len()..];
        let literal = js_string_literal_at(rest).and_then(|(_, len)| len);
        let spec = match literal {
            Some(len) => Some(&rest[..len]),
            None => rest.starts_with("__tt_spec)").then_some("__tt_spec"),
        };
        match spec {
            Some(spec) => out.push_str(&format!(
                "__ttImportReady(__dirname, {spec}).then(() => __toESM(require("
            )),
            None => out.push_str(LOWERED),
        }
    }
    out.push_str(rest);
    out
}

/// Module-runner: is it enabled? (default on; TURBO_NO_MR disables → legacy bundle path)
fn mr_enabled() -> bool {
    std::env::var("TURBO_NO_MR").is_err()
}

// E12 (shipped v0.2.14): memoize nearest_tsconfig + resolve_spec_as. Default ON — a clean win on
// both suites (ui full −7.4%, payroll full −13.9%, identical pass/fail). Escape hatch TURBO_NO_E12
// mirrors TURBO_NO_CODE_CACHE. (E6 transform-existence memo = neutral, E11 drain-loop hoist =
// slightly slower — both measured no-help at the validated jobs=8 protocol and not shipped.)
fn e12_enabled() -> bool {
    std::env::var("TURBO_NO_E12").is_err()
}

/// Bundle a node_modules entry to CJS (one esbuild call per package entry), with react family
/// externalized so it's shared. Returns the bundled code. Cached by entry+mtime.
fn esbuild_bundle_dep_cjs(abs: &Path) -> Option<String> {
    let root = project_root(abs)?;
    let esbuild = esbuild_bin(&root);
    use std::hash::{Hash, Hasher};
    let mtime = std::fs::metadata(abs)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    abs.hash(&mut h);
    mtime.hash(&mut h);
    // v2: import.meta filled (turbo-test#123); v3: `__toESM` is runtime.js's `__ttToESM`
    // (turbo-test#122)
    // v4: package interop `__ttToESMDep`, `export *` of Node's names (turbo-test#133)
    // v5: package ES modules marked "pkg" (turbo-test#159)
    "mr-dep-v5".hash(&mut h);
    let cache = cache_dir().join(format!("mrdep-{:016x}.cjs", h.finish()));
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let mut cmd = std::process::Command::new(&esbuild);
    cmd.current_dir(&root).arg(abs).args([
        "--bundle",
        "--format=cjs",
        "--platform=browser",
        "--jsx=automatic",
        "--loader:.css=empty",
        "--loader:.scss=empty",
        "--loader:.svg=text",
        "--loader:.png=dataurl",
        "--loader:.jpg=dataurl",
        "--loader:.gif=dataurl",
        "--loader:.webp=dataurl",
        "--log-level=silent",
        "--define:process.env.NODE_ENV=globalThis.process.env.NODE_ENV",
        "--supported:dynamic-import=false",
        // Externalize EVERY bare import (generic — no hardcoded library names): each package
        // bundles only its own (relative) files and require()s its dependencies. So every
        // cross-package module — react, any React context (MUI ThemeContext, emotion cache),
        // any module-level singleton — resolves to ONE shared instance via the require cache.
        // Adding a new dependency needs no runner change; it shares the same way automatically.
        "--packages=external",
    ]);
    let out = cmd.stderr(std::process::Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    // Same postprocess as per-file transform: configurable export getters + the __toESM
    // identity override. The __toESM override is critical here too — without it, esbuild's
    // `__toESM(require(x), 1)` (node interop for ESM `import default`) sets `.default` to the
    // whole module, so `import styled from '@emotion/styled'` yields the exports object instead
    // of the styled function (breaking flux-ui etc.).
    let code = fill_import_meta(
        postprocess_mr_dep_cjs(String::from_utf8(out.stdout).ok()?, is_esm_module(abs)),
        Some(&root),
    );
    write_atomic(&cache, &code);
    Some(code)
}

/// The esbuild CJS transform for a single file, bypassing the native (oxc) emitter — used as the
/// retry when native output fails to compile in V8. App files go through `esbuild_transform_cjs`
/// (the same call `read_transformed` would make with native disabled); node_modules through the
/// esbuild package bundle. Returns the ready-to-wrap CJS, or `None` if esbuild can't produce it.
fn esbuild_cjs_fallback(abs: &Path, prefer_metadata: bool) -> Option<String> {
    if abs.components().any(|c| c.as_os_str() == "node_modules") {
        esbuild_bundle_dep_cjs(abs)
    } else {
        esbuild_transform_cjs(abs, prefer_metadata)
    }
}

fn read_transformed(abs: &Path, as_cjs: bool, prefer_metadata: bool) -> Option<String> {
    // Module-runner: per-module CJS transform (app) / per-package CJS bundle (node_modules)
    // so imports are live + mockable and react is shared. Only on the CJS load path (the test
    // entry, setup files + their require-graph); the legacy ESM path (DOM boot) stays oxc.
    if as_cjs && mr_enabled() {
        if let Some(decided) = read_transformed_mr(abs, prefer_metadata) {
            return decided;
        }
    }
    read_transformed_esm(abs)
}

/// The module-runner CJS transform of `read_transformed`: `Some(result)` when it decides the
/// load (a transform, or None under strict mode), `None` to fall through to the ESM path.
fn read_transformed_mr(abs: &Path, prefer_metadata: bool) -> Option<Option<String>> {
    // App code: transform per-file (live bindings → spyOn/vi.mock work). node_modules:
    // bundle-per-package (fast), but the context-bearing packages (react, @mui/*, @emotion/*)
    // are externalized from every bundle so their shared singletons (ThemeContext, emotion
    // cache, react) resolve to ONE instance via the require cache (no dual-context).
    if abs.components().any(|c| c.as_os_str() == "node_modules") {
        // Native package bundler for deps (P2b, default on); esbuild bundle is the fallback.
        // Under coverage the whole transform path stays on esbuild (see the app branch), so deps
        // do too — keeps coverage runs on the known-good esbuild path until native maps reach
        // parity.
        if crate::esm_cjs::deps_enabled() && !crate::coverage::enabled() {
            if let Some(code) = native_dep_cjs(abs) {
                return Some(Some(code));
            }
            if crate::esm_cjs::strict() {
                return Some(None);
            }
        }
        esbuild_bundle_dep_cjs(abs).map(Some)
    } else {
        read_transformed_mr_app(abs, prefer_metadata)
    }
}

/// `read_transformed_mr` for an app (non-node_modules) module.
fn read_transformed_mr_app(abs: &Path, prefer_metadata: bool) -> Option<Option<String>> {
    // emitDecoratorMetadata files MUST take the project's TypeScript (tsc) path on the
    // FIRST load, not oxc. oxc has no type checker: for a decorated field typed as an
    // imported INTERFACE (e.g. Sequelize/Mongoose `@Column() declare x: SomeIface`), it
    // emits `Reflect.metadata("design:type", SomeIface)` — referencing the type-only import
    // as a value, so it can't be elided. The interface has no runtime export, so V8 fails
    // ESM instantiation with `does not provide an export named SomeIface`. tsc (no checker
    // either, isolatedModules) emits a guarded read instead, `typeof SomeIface ===
    // "function" ? SomeIface : Object`; after esbuild's ESM->CJS pass that is a property
    // read of the required module, so a missing export gives `Object` (ts-jest parity).
    // This fails at LINK time (before eval), so the eval-throw retry-on-load can't rescue
    // it — hence routing here, up front.
    // node_modules excluded (ship pre-compiled JS); browser/React projects have no
    // emitDecoratorMetadata so this branch never fires for them.
    let needs_meta = !abs.components().any(|c| c.as_os_str() == "node_modules")
        && decorator_metadata_enabled(abs)
        && std::fs::read_to_string(abs)
            .map(|s| file_has_decorator(&s))
            .unwrap_or(false);
    if needs_meta && !crate::coverage::enabled() {
        if let Some(code) = esbuild_transform_cjs(abs, true) {
            return Some(Some(code));
        }
        // tsc/esbuild unavailable (e.g. a fixture without them) → fall through to oxc; it
        // still loads everything except interface-typed metadata, no worse than before.
    }
    // Native oxc ESM→CJS (P2a) for app files; esbuild fallback on any unhandled form.
    // Coverage stays on esbuild for now: the native single-pass emitter DOES append an inline
    // source map (see esm_cjs::emit), but oxc's codegen map is less dense than esbuild's
    // (fewer per-token mappings), so coverage.rs under-attributes inner functions/lines —
    // not yet parity.
    if crate::esm_cjs::enabled() && !crate::coverage::enabled() && !prefer_metadata {
        if let Some(decided) = read_transformed_native(abs) {
            return Some(decided);
        }
    }
    esbuild_transform_cjs(abs, prefer_metadata).map(Some)
}

/// The native oxc ESM→CJS transform of an app module (`Some` when it decides the load).
fn read_transformed_native(abs: &Path) -> Option<Option<String>> {
    if crate::location::is_entry(abs) {
        if let Some(code) = native_transform_cjs_located(abs) {
            return Some(Some(code));
        }
    }
    if let Some(code) = native_transform_cjs(abs) {
        if crate::location::frames_enabled() && !crate::location::is_entry(abs) {
            // failure frames (junit, the human reporters) in this module map back
            // too; its code runs below the CJS wrapper's first line. The map comes
            // from the same transform's lines, built only when a frame needs it.
            crate::location::set_module_map_lazy(abs, native_module_segments, 1);
        }
        return Some(Some(code));
    }
    // strict mode (conformity coverage measurement): do NOT fall back to esbuild on an
    // unhandled form — surface it as a load error so the harness counts it.
    if crate::esm_cjs::strict() {
        return Some(None);
    }
    None
}

/// `read_transformed`'s ESM path: oxc transform (source-mapped for located modules), cached.
fn read_transformed_esm(abs: &Path) -> Option<String> {
    let raw = with_binder(abs, read_app_source(abs)?, false);
    if !crate::transform::needs_transform(abs) {
        if crate::location::is_entry(abs) {
            crate::location::set_identity(abs);
        }
        return Some(raw);
    }
    // App `.ts`/`.tsx` files load through this path as ESM (load_graph → maybe_transform), NOT the
    // CJS native-emit branch above. So legacy-decorator lowering must also happen HERE, else a
    // NestJS / Sequelize-typescript / Mongoose class emits `export @Decorator class X {}` (oxc's
    // default decorator handling) → "Unexpected token 'export'", and circular decorated models trip
    // "Cannot access 'X' before initialization". Lower legacy decorators when the project enables
    // experimentalDecorators/emitDecoratorMetadata AND this file uses a decorator. node_modules are
    // excluded (they ship pre-compiled JS; their tsconfig isn't ours).
    let legacy_decorators = !abs.components().any(|c| c.as_os_str() == "node_modules")
        && decorator_metadata_enabled(abs)
        && file_has_decorator(&raw);
    if !legacy_decorators {
        if let Some(code) = transform_mapped_located(abs, &raw) {
            return Some(code);
        }
    }
    transform_esm_cached(abs, &raw, legacy_decorators)
}

/// A located module's transform with its source map kept (the entry, or any app module while
/// failure frames are printed); None when not located or the mapped transform fails.
fn transform_mapped_located(abs: &Path, raw: &str) -> Option<String> {
    // The test file under test locations: transform with a source map (no cache).
    if crate::location::is_entry(abs) {
        if let Ok((code, mappings)) = crate::transform::transform_mapped(abs, raw) {
            crate::location::set_map(abs, crate::coverage::mappings_segments(&mappings), raw);
            return Some(code);
        }
    }
    // Another app module while failure frames are printed (junit): keep its map too.
    if crate::location::frames_enabled()
        && !abs.components().any(|c| c.as_os_str() == "node_modules")
    {
        if let Ok((code, mappings)) = crate::transform::transform_mapped(abs, raw) {
            let map = crate::coverage::mappings_segments(&mappings);
            crate::location::set_module_map(abs, map, 0);
            return Some(code);
        }
    }
    None
}

/// The content-addressed cached oxc / tsc ESM transform of `read_transformed_esm`.
fn transform_esm_cached(abs: &Path, raw: &str, legacy_decorators: bool) -> Option<String> {
    // content-addressed disk cache, shared across runs and workers. Key on the legacy-decorator
    // decision so a config/file change can't serve stale (un-lowered) output. The tag changes
    // with the decorator transform (v2: tsc reads type-only imports through a namespace import),
    // so output cached by an older binary is not served.
    let mut key = cache_key(abs, raw);
    if legacy_decorators {
        key ^= 0x9e37_79b9_7f4a_7c16;
    }
    let path = cache_dir().join(format!("{:016x}.js", key));
    if let Ok(cached) = std::fs::read_to_string(&path) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(cached);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    // emitDecoratorMetadata on the ESM path: prefer the project's TypeScript (tsc_transform_esm),
    // not oxc. oxc has no type checker, so a decorated field typed as an imported INTERFACE
    // (`@Column() declare x: SomeIface`) makes it emit `Reflect.metadata("design:type", SomeIface)`
    // — the type-only import then can't be elided, and since the interface has no runtime export
    // the ESM linker fails: `does not provide an export named SomeIface`. transpileModule has no
    // checker either: it keeps such an import too (its guarded `typeof SomeIface` metadata read
    // still names the binding), which only links because tsc_transform_esm first moves imports
    // used solely as types onto a namespace import (`ns.SomeIface` → undefined → `Object`, as
    // ts-jest gives). It fails at LINK time, so the eval-throw retry-on-load can't rescue it —
    // route here, on first load. oxc remains the fallback when the project ships no typescript
    // (tsc_transform_esm → None).
    if legacy_decorators {
        // Use a TYPESCRIPT-based root (not project_root, which requires esbuild) — tsc_transform_esm
        // only needs `node_modules/typescript`, and many backends (ts-jest projects) ship typescript
        // but NOT esbuild. Without this the routing silently fell back to oxc, which keeps the
        // interface-typed import for its `design:type` reference → ESM link failure.
        if let Some(root) = tsc_root(abs) {
            if let Some(esm) = tsc_transform_esm(abs, &root) {
                let _ = std::fs::write(&path, &esm);
                return Some(esm);
            }
        }
    }
    let out = crate::transform::transform_with(abs, raw, legacy_decorators)
        .map_err(|e| {
            if loader_debug() {
                eprintln!("transform {}: {e}", abs.display());
            }
        })
        .ok()?;
    let _ = std::fs::write(&path, &out);
    Some(out)
}

// ---- vi.mock hoisting -----------------------------------------------------

/// Extract `vi.mock("spec", factory)` calls from source (string-literal specifier).
/// Returns (specifier, factory_source). Models vitest's hoisting: mocks are registered
/// before the module graph instantiates. Dynamic specifiers/automock: M-later.
fn extract_mocks(src: &str) -> Vec<(String, String)> {
    // Match both `vi.mock(` (vitest) and `jest.mock(` (jest) — same hoisting semantics.
    extract_mock_calls(src, ["vi.mock(", "jest.mock("])
}

/// The literal-specifier `vi.doMock` / `jest.doMock` calls of a file (not hoisted; they apply to
/// later imports), as (spec, factory) like extract_mocks.
fn extract_do_mocks(src: &str) -> Vec<(String, String)> {
    extract_mock_calls(src, ["vi.doMock(", "jest.doMock("])
}

/// `(spec, factory)` of every call to either needle with a string-literal first argument, in
/// source order: from the AST (mock_scope::mock_calls), or for source that does not parse by
/// scanning the text (balanced parentheses).
fn extract_mock_calls(src: &str, needles: [&str; 2]) -> Vec<(String, String)> {
    let callees = needles.map(|n| n.trim_end_matches('('));
    if let Some(calls) = crate::mock_scope::mock_calls(src, &callees) {
        return calls;
    }
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while let Some((rel, needle_len)) = src.get(i..).and_then(|s| {
        needles
            .iter()
            .filter_map(|n| s.find(n).map(|p| (p, n.len())))
            .min_by_key(|&(p, _)| p)
    }) {
        let start = i + rel + needle_len;
        // parse first arg: a string literal
        let rest = src.get(start..).unwrap_or("");
        let q = rest.trim_start();
        let lead = rest.len() - q.len();
        if q.is_empty() {
            break;
        }
        // (escapes decoded: an escaped quote does not end the specifier, turbo-test#42 item 4)
        let Some((spec, len)) = js_string_literal_at(q) else {
            i = start;
            continue;
        };
        let Some(len) = len else { break };
        // find factory: from after the spec's closing quote to the balanced ')'
        let mut j = start + lead + len;
        // the next token after the specifier decides: ',' -> a factory follows, anything else
        // (normally ')') -> no factory. A ',' further on belongs to later code (another call's
        // factory, a test callback), never to this call.
        let tail = src.get(j..).unwrap_or("");
        let ws = tail.len() - tail.trim_start().len();
        let mut factory = String::new();
        if tail[ws..].starts_with(',') {
            let fstart = j + ws + 1;
            // balance parens from fstart until matching close of vi.mock(
            let mut depth = 1i32; // we're inside vi.mock( ... )
            let mut k = fstart;
            while k < bytes.len() {
                match bytes[k] as char {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            factory = src.get(fstart..k).unwrap_or("").trim().to_string();
            j = k + 1;
        }
        out.push((spec, factory));
        i = j;
    }
    out
}

/// Register a mock from an already-evaluated exports value: expose as a synthetic module
/// (default + named keys) keyed by absolute path; all importers of that path get it.
/// Collect names imported from `abs` across bundle text: `import { a, b as c } from "abs"`
/// and `import D from "abs"` (default). Used to make mock synthetics vitest-lenient.
fn collect_named_imports(text: &str, abs: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle_variants = [format!("from\"{abs}\""), format!("from \"{abs}\"")];
    for needle in &needle_variants {
        let mut search = 0;
        while let Some(pos) = text.get(search..).and_then(|s| s.find(needle.as_str())) {
            let at = search + pos;
            search = at + needle.len();
            // look back for the `{ ... }` clause of THIS import statement only: a brace before
            // the statement's own `import` keyword belongs to an earlier import (a default-only
            // `import d from "<abs>"` has no clause of its own)
            let head = text.get(..at).unwrap_or("");
            let head = head
                .get(import_keyword_before(head).unwrap_or(0)..)
                .unwrap_or("");
            if let Some(close) = head.rfind('}') {
                if let Some(open) = head.get(..close).unwrap_or("").rfind('{') {
                    for part in head.get(open + 1..close).unwrap_or("").split(',') {
                        let name = part.split_whitespace().last().unwrap_or("").trim();
                        if !name.is_empty()
                            && name
                                .chars()
                                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                        {
                            out.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

/// Byte offset of the last `import` KEYWORD in `head` (not part of a longer identifier such as
/// `importer`).
fn import_keyword_before(head: &str) -> Option<usize> {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut end = head.len();
    while let Some(at) = head[..end].rfind("import") {
        let before = head[..at].chars().next_back();
        let after = head[at + "import".len()..].chars().next();
        if !before.is_some_and(ident) && !after.is_some_and(ident) {
            return Some(at);
        }
        end = at;
    }
    None
}

fn register_mock_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    exports_val: v8::Local<v8::Value>,
) -> Option<()> {
    let global = scope.get_current_context().global(scope);
    let mut names = object_keys(scope, global, exports_val);
    // names an import may name although the mock lacks them (runtime.js __ttMockNames)
    let extra = v8::String::new(scope, "__ttMockNames")
        .and_then(|k| global.get(scope, k.into()))
        .and_then(|f| v8::Local::<v8::Function>::try_from(f).ok())
        .and_then(|f| {
            let undefined = v8::undefined(scope).into();
            f.call(scope, undefined, &[exports_val])
        });
    if let Some(extra) = extra.filter(|v| v.is_array()) {
        let arr = v8::Local::<v8::Array>::try_from(extra).ok();
        for i in 0..arr.map_or(0, |a| a.length()) {
            if let Some(n) = arr.and_then(|a| a.get_index(scope, i)) {
                let n = n.to_rust_string_lossy(scope);
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
    }
    // Isolate-reuse: if this path was ALREADY loaded (e.g. a setup mock, or a real module a
    // cached importer holds a reference to), a test file re-mocking it must NOT swap in a new
    // exports object — cached CJS importers hold the old object and read its props live, so they
    // would keep seeing the old mock (e.g. the analytics trackEvent the test spies on differs
    // from the one a cached hook calls → "0 calls"). Instead, mutate the existing object in
    // place: copy the new mock's props onto it. Identity is preserved; every importer sees the
    // update. (Fresh mode reloads every module per file, so this only matters on the reuse path;
    // a fresh isolate under the reuse switch registers a new module, as vitest does.)
    if on_reuse_path() && remock_in_place(scope, abs, exports_val, &names) {
        return Some(());
    }
    // union in names requested by importers but missing from the factory (lenient mock)
    REGISTRY.with(|r| {
        if let Some(extra) = r.borrow().extra_exports.get(abs) {
            for n in extra {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
    });
    let exports_global = v8::Global::new(scope, exports_val);
    let synth = make_synthetic(scope, &names)?;
    let hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.cjs_exports.insert(abs.to_path_buf(), exports_global);
        reg.cjs_export_names.insert(abs.to_path_buf(), names);
        reg.cjs_synth_by_path
            .insert(abs.to_path_buf(), synth_global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
        reg.mocks.insert(abs.to_path_buf(), String::new());
    });
    if on_reuse_path() {
        invalidate_importers(abs);
    }
    Some(())
}

/// `register_mock_value`'s reuse-path re-mock of an already-loaded module: copy the new mock's
/// props onto the existing exports object (true when done).
fn remock_in_place<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    exports_val: v8::Local<v8::Value>,
    names: &[String],
) -> bool {
    {
        if let Some(existing_g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(abs).cloned()) {
            let existing = v8::Local::new(scope, &existing_g);
            if existing.is_object() && exports_val.is_object() {
                if let (Some(dst), Some(src)) =
                    (existing.to_object(scope), exports_val.to_object(scope))
                {
                    for n in names {
                        if let Some(k) = v8::String::new(scope, n) {
                            // (a __ttMockNames name the mock lacks is not copied)
                            if !src.has(scope, k.into()).unwrap_or(false) {
                                continue;
                            }
                            if let Some(val) = src.get(scope, k.into()) {
                                dst.set(scope, k.into(), val);
                            }
                        }
                    }
                    REGISTRY.with(|r| {
                        r.borrow_mut()
                            .mocks
                            .insert(abs.to_path_buf(), String::new())
                    });
                    invalidate_importers(abs);
                    return true;
                }
            }
        }
    }
    false
}

/// Register a synthetic module for `abs` by running a generated, self-contained factory source
/// (`() => (...)`): the asset / JSON stubs load_dep serves. (Test-file mock factories are run by
/// the runtime's vi.mock, which handles factory errors and async factories.)
fn register_mock<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    factory_src: &str,
) -> Option<()> {
    let code = v8::String::new(scope, &format!("({factory_src})"))?;
    let script = v8::Script::compile(scope, code, None)?;
    let f = v8::Local::<v8::Function>::try_from(script.run(scope)?).ok()?;
    let undef = v8::undefined(scope).into();
    let exports_val = f.call(scope, undef, &[])?;
    register_mock_value(scope, abs, exports_val)
}

/// When `abs` is mocked by a factory that threw, the error importing it must throw (runtime.js
/// registered its error stand-in for it; `__ttMockFactoryError` maps that back to the error).
fn mock_factory_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Value>> {
    let g = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.mocks
            .contains_key(abs)
            .then(|| reg.cjs_exports.get(abs).cloned())
            .flatten()
    })?;
    let exports = v8::Local::new(scope, &g);
    let global = scope.get_current_context().global(scope);
    let k = v8::String::new(scope, "__ttMockFactoryError")?;
    let f = v8::Local::<v8::Function>::try_from(global.get(scope, k.into())?).ok()?;
    let undef = v8::undefined(scope).into();
    let err = f.call(scope, undef, &[exports])?;
    (!err.is_undefined()).then_some(err)
}

/// Drain globalThis.__pendingMocks (filled by runtime vi.mock during a module's execution),
/// registering each mock keyed by its specifier resolved relative to `base_dir`. Clears the
/// queue afterward.
/// Await async mock factories (globalThis.__resolvePendingMocks) by pumping microtasks
/// until the returned promise settles, so the queue holds resolved exports before draining.
fn resolve_pending_mocks(scope: &mut v8::PinScope) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__resolvePendingMocks").unwrap();
    let Some(f) = global
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
    else {
        return;
    };
    let recv = v8::undefined(scope).into();
    // Guard the pump: a factory's `await import('<self>')` re-enters load_cjs, whose nested drain
    // would otherwise clear the queue mid-resolve and register the still-pending promise.
    let prev = RESOLVING_MOCKS.with(|f| f.replace(true));
    let Some(ret) = f.call(scope, recv, &[]) else {
        RESOLVING_MOCKS.with(|f| f.set(prev));
        return;
    };
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(ret) {
        for _ in 0..100000 {
            if promise.state() != v8::PromiseState::Pending {
                break;
            }
            let _ = call_global_bool(scope, global, "__drainNextTicks");
            scope.perform_microtask_checkpoint();
            // also run a due macrotask: async factories that `await import(...)` resolve their
            // dynamic imports across macrotask turns, not just microtasks.
            let _ = call_global_bool(scope, global, "__stepMacro");
            scope.perform_microtask_checkpoint();
        }
    }
    RESOLVING_MOCKS.with(|f| f.set(prev));
}

fn drain_pending_mocks(scope: &mut v8::PinScope, base_dir: &Path, skip_relative: bool) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__pendingMocks").unwrap();
    let arr = match global
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
    {
        Some(a) => a,
        None => return,
    };
    let spec_key = v8::String::new(scope, "spec").unwrap();
    let exp_key = v8::String::new(scope, "exports").unwrap();
    // Entries whose factory promise is still unresolved must NOT be registered here and must be
    // kept in the queue for the outer resolve+drain. This guards re-entrancy: an async factory's
    // `await import('<self>')` loads the real module via load_cjs(drain_mocks=true), which drains
    // pending mocks WHILE the factory's own promise is still pending. Registering that raw promise
    // would mark the module mocked with an empty (names=[]) exports, so the real module wins and
    // the test file's named imports never rebind to the factory's spies (the payroll-app
    // `vi.mocked(<named import>)` is undefined / points at the real export bug).
    let mut retained: Vec<v8::Local<v8::Value>> = Vec::new();
    for i in 0..arr.length() {
        let item = arr
            .get_index(scope, i)
            .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok());
        let Some(item) = item else {
            continue;
        };
        let Some(spec) = item
            .get(scope, spec_key.into())
            .map(|v| v.to_rust_string_lossy(scope))
        else {
            continue;
        };
        let _ = skip_relative;
        let exports = item
            .get(scope, exp_key.into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        if exports.is_promise() || is_thenable(scope, exports) {
            retained.push(item.into()); // unresolved factory — defer to the outer drain
            continue;
        }
        if exports.is_undefined() {
            continue; // a factory that returned nothing (a throwing one left its error stand-in)
        }
        if let Some(key) = mock_key(&spec, base_dir) {
            register_mock_value(scope, &key, exports);
        }
    }
    // rebuild the queue with only the still-pending entries (cleared if none remain)
    let next = v8::Array::new(scope, retained.len() as i32);
    for (i, item) in retained.into_iter().enumerate() {
        next.set_index(scope, i as u32, item);
    }
    global.set(scope, key.into(), next.into());
}

/// True if `val` is a non-promise object exposing a callable `then` (a thenable). Promises are
/// detected separately via `is_promise()`; both kinds must be deferred, not registered as mocks.
fn is_thenable(scope: &mut v8::PinScope, val: v8::Local<v8::Value>) -> bool {
    let Ok(obj) = v8::Local::<v8::Object>::try_from(val) else {
        return false;
    };
    let then = v8::String::new(scope, "then");
    then.and_then(|k| obj.get(scope, k.into()))
        .is_some_and(|v| v.is_function())
}

// ---- CommonJS -------------------------------------------------------------

/// V8 bytecode (code) cache for compiled CJS module wrappers. A fresh isolate per test file
/// otherwise re-parses+re-compiles every required module (incl. node_modules barrels) from
/// scratch. V8 can serialize a script's compiled bytecode; we persist it keyed by the exact
/// wrapped source and consume it on later compiles (any isolate, any run/worker). On a version
/// or content mismatch V8 marks the cached data rejected and recompiles — always safe. Generic:
/// helps any suite that loads the same modules across many isolates. ON by default (measured
/// ~1.5-2% faster, identical pass/fail on payroll+ui); disable with TURBO_NO_CODE_CACHE. Disabled
/// under coverage (that path attaches a named origin + inspector and runs its own isolate).
fn code_cache_enabled() -> bool {
    std::env::var("TURBO_NO_CODE_CACHE").is_err() && !crate::coverage::enabled()
}

fn cc_path(wrapped: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    wrapped.hash(&mut h);
    // v2: caches are produced after the test file ran (they carry its lazily compiled functions)
    "cc-v2".hash(&mut h);
    cache_dir().join(format!("cc-{:016x}.bin", h.finish()))
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) {
    let seq = ATOMIC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{}", std::process::id(), seq));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// A module's wrapped CJS source and its bytecode cache, shared by every isolate in the process
/// (from alphajew420's #106). A fresh isolate per file loads react, the testing library and the
/// app's modules again each time; each load read the transform cache, wrapped the source (a
/// copy), hashed it for its bytecode cache, read that file and copied the text into the V8 heap.
/// A node_modules module's ASCII text is leaked on purpose: V8 uses it as an external one-byte
/// string (no copy, not counted in the isolate's heap) that may outlive the entry; other text is
/// owned ([`shared_module_source`]). One entry per (path, mode): an edited file replaces its
/// entry, and app-module entries are capped (LRU). A source only one load uses is owned and freed
/// with its last user ([`module_source_for`]).
struct ModuleSource {
    wrapped: std::borrow::Cow<'static, str>,
    /// The leaked ASCII text's bytes, for an external string; `None` for owned text.
    one_byte: Option<&'static [u8]>,
    cc_path: PathBuf,
    /// The size of the richest bytecode cache this process read or wrote for it (0: none or
    /// rejected). The bytes themselves stay in the file, read on each consume.
    cc: std::sync::Mutex<usize>,
}

#[cfg(test)]
impl ModuleSource {
    /// The size of the richest bytecode cache seen for it.
    fn cc_len(&self) -> usize {
        *self.cc.lock().unwrap()
    }
}

/// (path, decorator-metadata mode)
type SourceKey = (PathBuf, bool);

/// What identifies a version of a file: mtime and ctime in ns, size, inode, and for a file
/// modified in the last 2 s (a coarse-mtime filesystem can give an edit the same mtime and size)
/// a hash of its bytes.
type SourceStamp = (u128, u128, u64, u64, u64);

fn source_stamp(abs: &Path) -> Option<SourceStamp> {
    let meta = std::fs::metadata(abs).ok()?;
    let ns = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos())
    };
    let mtime = ns(meta.modified());
    #[cfg(unix)]
    let (ino, ctime) = {
        use std::os::unix::fs::MetadataExt;
        (
            meta.ino(),
            u128::try_from(meta.ctime()).unwrap_or(0) * 1_000_000_000
                + u128::try_from(meta.ctime_nsec()).unwrap_or(0),
        )
    };
    #[cfg(not(unix))]
    let (ino, ctime) = (0, 0);
    let now = ns(Ok(std::time::SystemTime::now()));
    stamp_from(mtime, ctime, meta.len(), ino, now, || {
        std::fs::read(abs).ok()
    })
}

/// A [`SourceStamp`] from a file's metadata, the time now (both in ns) and its bytes, read only
/// for a file modified less than 2 s ago (or with a future mtime); `None` when that read fails.
/// A stamp taken after the window has hash 0: an entry made inside the window then misses once
/// and reloads, and an edit that keeps an older mtime and the size is still caught by ctime and
/// inode, not by the hash.
fn stamp_from(
    mtime: u128,
    ctime: u128,
    len: u64,
    ino: u64,
    now: u128,
    bytes: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<SourceStamp> {
    let hash = if now.saturating_sub(mtime) < 2_000_000_000 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes()?.hash(&mut h);
        h.finish() | 1
    } else {
        0
    };
    Some((mtime, ctime, len, ino, hash))
}

const CJS_WRAPPER_OPEN: &str = "(function (exports, module, require, __filename, __dirname) {\n";

/// Whether a module's transform depends only on its file, so its wrapped source can be shared:
/// not the test file or a setup file (their factories get a binder, turbo-test#205; the test
/// file's transform also feeds test locations), not an app module whose source map failure
/// frames keep, not under coverage. A module the shared cache skips loads as before, with the
/// transform's per-file side effects.
fn shareable_source(abs: &Path) -> bool {
    let app = !abs.components().any(|c| c.as_os_str() == "node_modules");
    !crate::coverage::enabled()
        && !TEST_FILE.with(|f| *f.borrow() == abs)
        && !SCOPED_SETUP_FILES.with(|f| f.borrow().iter().any(|s| s == abs))
        && !(app && crate::location::frames_enabled())
}

/// The module's wrapped source: the process's shared copy when [`shareable_source`], else an
/// owned one of its own, freed once its load and pending cache write are done (V8 copies its
/// text, so nothing outlives it).
fn module_source_for(abs: &Path, prefer_metadata: bool) -> Option<Arc<ModuleSource>> {
    if shareable_source(abs) {
        return module_source(abs, prefer_metadata);
    }
    let raw = read_transformed(abs, true, prefer_metadata)?;
    Some(new_module_source(format!("{CJS_WRAPPER_OPEN}{raw}\n}})")))
}

/// An owned source (its text is copied into V8).
fn new_module_source(wrapped: String) -> Arc<ModuleSource> {
    Arc::new(ModuleSource {
        cc_path: cc_path(&wrapped),
        wrapped: wrapped.into(),
        one_byte: None,
        cc: std::sync::Mutex::new(0),
    })
}

/// A shared source. A node_modules module's ASCII text is leaked so V8 can use it as an external
/// one-byte string (no copy, outside the isolate's heap, and V8 may hold it past the entry); any
/// other text is owned and copied into V8, so it is freed with its entry.
fn shared_module_source(wrapped: String, external: bool) -> Arc<ModuleSource> {
    let cc_path = cc_path(&wrapped);
    let (wrapped, one_byte): (std::borrow::Cow<'static, str>, _) = if external && wrapped.is_ascii()
    {
        let text: &'static str = Box::leak(wrapped.into_boxed_str());
        (text.into(), Some(text.as_bytes()))
    } else {
        (wrapped.into(), None)
    };
    Arc::new(ModuleSource {
        wrapped,
        one_byte,
        cc_path,
        cc: std::sync::Mutex::new(0),
    })
}

/// A shared entry: the file version it was made from, its source, when it was last used (a
/// process-wide tick) and whether it is an app module (outside node_modules: capped).
struct SharedSource {
    stamp: SourceStamp,
    source: Arc<ModuleSource>,
    last_use: u64,
    app: bool,
}

/// At most this many shared app-module entries; past it the least recently used goes.
/// node_modules entries are not capped (a suite's dependency set is bounded).
const MAX_SHARED_APP_SOURCES: usize = 2048;

type SourceMap = std::sync::Mutex<HashMap<SourceKey, SharedSource>>;

fn shared_sources() -> &'static SourceMap {
    static SOURCES: std::sync::OnceLock<SourceMap> = std::sync::OnceLock::new();
    SOURCES.get_or_init(Default::default)
}

/// Evict the least recently used app entries until at most `cap` are left.
fn cap_app_sources(map: &mut HashMap<SourceKey, SharedSource>, cap: usize) {
    let mut apps: Vec<(u64, SourceKey)> = map
        .iter()
        .filter(|(_, e)| e.app)
        .map(|(k, e)| (e.last_use, k.clone()))
        .collect();
    if apps.len() <= cap {
        return;
    }
    apps.sort_unstable_by_key(|(t, _)| *t);
    let excess = apps.len() - cap;
    for (_, k) in apps.into_iter().take(excess) {
        map.remove(&k);
    }
}

/// How many shared entries a path has (one per decorator-metadata mode at most).
#[cfg(test)]
fn module_source_entries(abs: &Path) -> usize {
    shared_sources()
        .lock()
        .unwrap()
        .keys()
        .filter(|k| k.0 == abs)
        .count()
}

fn module_source(abs: &Path, prefer_metadata: bool) -> Option<Arc<ModuleSource>> {
    static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = source_stamp(abs)?;
    let key: SourceKey = (abs.to_path_buf(), prefer_metadata);
    let map = shared_sources();
    let hit = |m: &mut HashMap<SourceKey, SharedSource>| {
        let e = m.get_mut(&key).filter(|e| e.stamp == stamp)?;
        e.last_use = TICK.fetch_add(1, Ordering::Relaxed);
        Some(Arc::clone(&e.source))
    };
    if let Some(src) = hit(&mut map.lock().unwrap()) {
        return Some(src);
    }
    // transformed outside the lock (it can run esbuild); a racing thread's copy is dropped
    let raw = read_transformed(abs, true, prefer_metadata)?;
    let wrapped = format!("{CJS_WRAPPER_OPEN}{raw}\n}})");
    let mut map = map.lock().unwrap();
    if let Some(src) = hit(&mut map) {
        return Some(src);
    }
    let app = !abs.components().any(|c| c.as_os_str() == "node_modules");
    let source = shared_module_source(wrapped, !app);
    // an edited file replaces its entry (leaked external text stays: V8 may still use it)
    map.insert(
        key,
        SharedSource {
            stamp,
            source: Arc::clone(&source),
            last_use: TICK.fetch_add(1, Ordering::Relaxed),
            app,
        },
    );
    if app {
        cap_app_sources(&mut map, MAX_SHARED_APP_SOURCES);
    }
    Some(source)
}

/// The module's source as a V8 string: external (no copy) for leaked ASCII text.
fn source_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &ModuleSource,
) -> Option<v8::Local<'s, v8::String>> {
    match source.one_byte {
        Some(b) => v8::String::new_external_onebyte_static(scope, b),
        None => v8::String::new(scope, &source.wrapped),
    }
}

/// The module's bytecode cache, read from its file (the slot keeps only the richest size).
fn module_code_cache(module: &ModuleSource) -> Option<Vec<u8>> {
    let bytes = std::fs::read(&module.cc_path)
        .ok()
        .filter(|b| !b.is_empty())?;
    let mut slot = module.cc.lock().unwrap();
    *slot = (*slot).max(bytes.len());
    Some(bytes)
}

/// Persist a new bytecode cache, for the process's other isolates too, unless it holds no more
/// than the current one or (for a cache-hit refresh) the `consumed` one: checked under the slot's
/// lock, so a poorer cache never replaces a richer one.
fn store_code_cache(module: &ModuleSource, bytes: &[u8], consumed: Option<usize>) {
    let mut slot = module.cc.lock().unwrap();
    if bytes.len() <= (*slot).max(consumed.unwrap_or(0)) {
        return;
    }
    write_atomic_bytes(&module.cc_path, bytes);
    *slot = bytes.len();
}

/// Forget a cache V8 rejected (another V8 build or flag set): the slot and its file, so no
/// other isolate consumes it again.
fn reject_code_cache(module: &ModuleSource) {
    let mut slot = module.cc.lock().unwrap();
    *slot = 0;
    let _ = std::fs::remove_file(&module.cc_path);
}

/// Compile the wrapped CJS source, consuming a persisted bytecode cache when present and
/// producing one on a miss. Returns the bound Script. Falls back to a plain compile on any
/// cache miss/reject (V8 recompiles internally and we keep that result).
fn compile_cjs_cached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: v8::Local<v8::String>,
    module: &Arc<ModuleSource>,
    origin: Option<&v8::ScriptOrigin>,
) -> Option<v8::Local<'s, v8::Script>> {
    use v8::script_compiler as sc;
    if let Some(bytes) = module_code_cache(module) {
        let cached = v8::CachedData::new(&bytes);
        let mut source = sc::Source::new_with_cached_data(code, origin, cached);
        let (consume, reason) = (
            sc::CompileOptions::ConsumeCodeCache,
            sc::NoCacheReason::NoReason,
        );
        let compiled = sc::compile(scope, &mut source, consume, reason);
        // `bytes` stays alive through this block (Source borrowed it for the compile).
        // A rejected cache (another V8 build or flag set) still compiles; rewrite it then so
        // later loads hit instead of recompiling from source forever.
        let rejected = source.get_cached_data().is_none_or(|c| c.rejected());
        if let Some(s) = compiled {
            if rejected {
                reject_code_cache(module);
                defer_code_cache(scope, module, s, None);
            } else if refresh_due(&module.cc_path) {
                defer_code_cache(scope, module, s, Some(bytes.len()));
            }
            return Some(s);
        }
    }
    // Miss/reject: compile fresh and persist the bytecode for next time.
    let mut source = sc::Source::new(code, origin);
    let (options, reason) = (
        sc::CompileOptions::NoCompileOptions,
        sc::NoCacheReason::NoReason,
    );
    let script = sc::compile(scope, &mut source, options, reason)?;
    defer_code_cache(scope, module, script, None);
    Some(script)
}

thread_local! {
    /// Scripts whose caches are written once the test file ran ([`flush_code_caches`]), with
    /// their cache paths and the size of the cache they consumed: `None` for a script compiled
    /// without a usable cache (always written), `Some(n)` for a cache-hit refresh (written only
    /// when the run compiled more of it, so the new cache is larger than `n`).
    static PENDING_CODE_CACHES: RefCell<Vec<PendingCodeCache>> = const { RefCell::new(Vec::new()) };
    /// Cache hits per cache path in this worker, for [`refresh_due`]'s backoff.
    static CODE_CACHE_HITS: RefCell<HashMap<PathBuf, u32>> = RefCell::new(HashMap::new());
}

type PendingCodeCache = (
    Arc<ModuleSource>,
    v8::Global<v8::UnboundScript>,
    Option<usize>,
);

/// Whether this cache hit re-checks the cache for functions compiled since it was written: on
/// the 1st, 2nd, 4th, 8th... hit of a path in this worker. A function first called by a later
/// test file is compiled lazily after the cache was made; without a refresh every later run
/// compiles it again. The backoff bounds the serialization cost to log2(hits) per module.
fn refresh_due(path: &Path) -> bool {
    CODE_CACHE_HITS.with(|h| {
        let mut h = h.borrow_mut();
        let n = h.entry(path.to_path_buf()).or_insert(0);
        *n += 1;
        n.is_power_of_two()
    })
}

/// Queue `script`'s cache for [`flush_code_caches`]: `consumed` is the size of the cache it
/// was compiled from for a cache-hit refresh, `None` for a script compiled without a usable one.
fn defer_code_cache(
    scope: &mut v8::PinScope,
    module: &Arc<ModuleSource>,
    script: v8::Local<v8::Script>,
    consumed: Option<usize>,
) {
    let unbound = v8::Global::new(scope, script.get_unbound_script(scope));
    PENDING_CODE_CACHES.with(|p| p.borrow_mut().push((Arc::clone(module), unbound, consumed)));
}

/// Drop the pending caches a previous file left (it failed to load, so it never reached
/// [`flush_code_caches`]). Their scripts may belong to a disposed isolate: touching or dropping
/// those handles panics, so they are forgotten.
fn discard_stale_code_caches() {
    std::mem::forget(PENDING_CODE_CACHES.with(|p| std::mem::take(&mut *p.borrow_mut())));
}

/// Drop the caches a file left pending (it did not reach [`flush_code_caches`]) while its
/// isolate is alive, so none is left for a later file or the thread-local destructor, which
/// runs after the isolate is gone (turbo-test#228 at a worker's end). The guard covers an
/// unwinding run: it forgets what is left instead.
fn drop_pending_code_caches() {
    drop(PENDING_CODE_CACHES.with(|p| std::mem::take(&mut *p.borrow_mut())));
}

struct PendingCodeCacheGuard;

impl Drop for PendingCodeCacheGuard {
    fn drop(&mut self) {
        discard_stale_code_caches();
    }
}

/// Write the caches of the scripts this test file compiled from source, and refresh the cache
/// hits that compiled more functions than their cache held. Produced after the run, a cache also
/// holds every function the file called: V8 compiles functions lazily, and a cache made right
/// after the top-level compile has none of them, so each later isolate would parse and compile
/// them again.
fn flush_code_caches(scope: &mut v8::PinScope) {
    for (module, unbound, consumed) in
        PENDING_CODE_CACHES.with(|p| std::mem::take(&mut *p.borrow_mut()))
    {
        if let Some(cc) = v8::Local::new(scope, &unbound).create_code_cache() {
            store_code_cache(&module, &cc, consumed);
        }
    }
}

fn load_cjs<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path, drain_mocks: bool) -> Option<()> {
    load_cjs_inner(scope, abs, drain_mocks, false)
}

fn load_cjs_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    drain_mocks: bool,
    prefer_metadata: bool,
) -> Option<()> {
    if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(abs)) {
        return Some(());
    }
    let wrapper = compile_cjs_wrapper(scope, abs, prefer_metadata)?;
    let (exports_obj, module_obj, exports_key) = build_cjs_module_object(scope)?;
    let dir = abs.parent().unwrap();
    let dir_str = v8::String::new(scope, &dir.to_string_lossy())?;
    let global = scope.get_current_context().global(scope);
    let filename = v8::String::new(scope, &abs.to_string_lossy())?;
    let require = cjs_require_for(scope, global, abs, dir_str, filename)?;
    // expose this module's dir so runtime vi.mock() (hoisted above the requires) can resolve a
    // relative spec + register immediately, before the module's consumer requires run.
    let prev_dir = v8::String::new(scope, "__ttDir").and_then(|k| {
        let prev = global.get(scope, k.into());
        global.set(scope, k.into(), dir_str.into());
        prev
    });
    let (call_ok, body_exc) = run_cjs_body_scope(
        scope,
        abs,
        wrapper,
        module_obj,
        exports_obj,
        require,
        filename,
        dir_str,
    );
    restore_tt_dir(scope, global, prev_dir);
    if !call_ok {
        return handle_cjs_body_throw(scope, abs, drain_mocks, prefer_metadata, body_exc);
    }
    settle_module_mocks(scope, abs, drain_mocks);
    let final_exports = module_obj.get(scope, exports_key.into())?;
    register_cjs_exports(scope, global, abs, final_exports)
}

/// Read and compile a CJS module's wrapper function (`(function (exports, module, …) { … })`).
/// Throws vite's transform / compile error on the scope when it fails, mirroring the native
/// load path.
fn compile_cjs_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    prefer_metadata: bool,
) -> Option<v8::Local<'s, v8::Function>> {
    let Some(module) = module_source_for(abs, prefer_metadata) else {
        throw_transform_failure(scope, abs);
        return None;
    };
    let code = source_string(scope, &module)?;
    // Under --coverage, give the script an origin named after the file so V8's precise-coverage
    // report carries a URL we can map back to source. Off by default (origin None) — zero change.
    // (compiled script, the compile error when neither the native output nor the esbuild retry
    // compiles: re-thrown below, outside this TryCatch, as the load's cause)
    let (script, compile_exc) = compile_cjs_module(scope, abs, &module, code, prefer_metadata);
    throw_compile_exc(scope, abs, compile_exc);
    let script = v8::Local::new(scope, &script?);
    v8::Local::<v8::Function>::try_from(script.run(scope)?).ok()
}

/// Build the CJS `module` / `module.exports` object pair plus the `"exports"` key.
fn build_cjs_module_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::String>,
)> {
    let exports_obj = v8::Object::new(scope);
    let module_obj = v8::Object::new(scope);
    let exports_key = v8::String::new(scope, "exports")?;
    module_obj.set(scope, exports_key.into(), exports_obj.into());
    Some((exports_obj, module_obj, exports_key))
}

/// Run the CJS body inside the loading / require-stack bookkeeping.
#[allow(clippy::too_many_arguments)]
fn run_cjs_body_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    wrapper: v8::Local<'s, v8::Function>,
    module_obj: v8::Local<'s, v8::Object>,
    exports_obj: v8::Local<'s, v8::Object>,
    require: v8::Local<'s, v8::Value>,
    filename: v8::Local<'s, v8::String>,
    dir_str: v8::Local<'s, v8::String>,
) -> (bool, Option<v8::Global<v8::Value>>) {
    // mark in-progress (with its module object) so a circular require() returns live exports
    let module_global = v8::Global::new(scope, module_obj);
    REGISTRY.with(|r| {
        r.borrow_mut()
            .loading
            .insert(abs.to_path_buf(), module_global)
    });
    REQUIRE_STACK.with(|st| st.borrow_mut().push(abs.to_path_buf()));
    // (body result, the body's exception: re-thrown below unless the decorator retry runs)
    let out = call_cjs_body(
        scope,
        abs,
        wrapper,
        &[
            exports_obj.into(),
            module_obj.into(),
            require,
            filename.into(),
            dir_str.into(),
        ],
    );
    REGISTRY.with(|r| r.borrow_mut().loading.remove(abs));
    REQUIRE_STACK.with(|st| st.borrow_mut().pop());
    out
}

/// The module body threw. If it is a decorator file in an `emitDecoratorMetadata` project and we
/// have not already retried, re-transform THIS file via the metadata path (ts.transpileModule)
/// and re-run once — the throw is often a missing-`design:type` error (Mongoose `@Prop` /
/// Sequelize `@Column` read metadata at class-definition). Otherwise re-throw the module's own
/// error so `require()` / the importing module / the entry load fail with it (Node/vitest), not
/// with a generic "failed to load" that has no cause.
fn handle_cjs_body_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    drain_mocks: bool,
    prefer_metadata: bool,
    body_exc: Option<v8::Global<v8::Value>>,
) -> Option<()> {
    if !prefer_metadata && wants_metadata_retry(abs) {
        return load_cjs_inner(scope, abs, drain_mocks, true);
    }
    if let Some(e) = body_exc {
        let e = v8::Local::new(scope, &e);
        scope.throw_exception(e);
    }
    None
}

/// Compile `load_cjs_inner`'s wrapped module (named origin when coverage / locations need it),
/// retrying once through esbuild when the native output does not compile: the script, else the
/// compile error.
fn compile_cjs_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    module: &Arc<ModuleSource>,
    code: v8::Local<v8::String>,
    prefer_metadata: bool,
) -> (
    Option<v8::Global<v8::Script>>,
    Option<v8::Global<v8::Value>>,
) {
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    // Test locations (location.rs) find the test file's frames by this name too.
    let located = crate::location::is_entry(abs);
    if located {
        crate::location::set_offset(abs, 1);
    }
    // (failure frames name an app module's file too: junit, turbo-test#81)
    let framed = crate::location::frames_enabled()
        && !abs.components().any(|c| c.as_os_str() == "node_modules");
    let origin = if crate::coverage::enabled() || located || framed {
        v8::String::new(tc, &abs.to_string_lossy()).map(|name| script_origin(tc, name))
    } else {
        None
    };
    // A named origin (the entry file, framed app modules) shares the bytecode cache: the
    // cached data does not depend on the script's name, and V8 rejects a mismatch safely.
    let compiled = if code_cache_enabled() {
        compile_cjs_cached(tc, code, module, origin.as_ref())
    } else {
        match &origin {
            Some(o) => v8::Script::compile(tc, code, Some(o)),
            None => v8::Script::compile(tc, code, None),
        }
    };
    match compiled {
        Some(s) => (Some(v8::Global::new(tc, s)), None),
        None => {
            // The native (oxc) emitter can occasionally produce JS that V8 rejects at compile —
            // a transform edge the esbuild path handles cleanly. Because it fails at COMPILE
            // (not transform) time, the transform-level esbuild fallback in read_transformed was
            // never taken. Retry this one file through esbuild and recompile rather than
            // hard-load-erroring. esbuild is the proven fallback (same output read_transformed
            // uses when native is disabled), so a file that only the native path mangled now
            // loads; a genuinely-broken source still fails (esbuild can't compile it either).
            let native_err = tc.exception().map(|e| v8::Global::new(tc, e));
            let fb = esbuild_fallback_script(tc, abs, module, prefer_metadata);
            if fb.is_none() {
                debug_cjs_compile_failure(tc, abs);
                return (None, native_err);
            }
            (fb, None)
        }
    }
}

/// Compile `abs` through esbuild (the pre-transform CJS source) and return its script, or
/// `None` when esbuild produces the same wrapped text we already tried (output identical) or
/// esbuild itself compiles nothing. Used only as a fallback inside `compile_cjs_module`.
fn esbuild_fallback_script(
    tc: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope<'_>>>,
    abs: &Path,
    module: &Arc<ModuleSource>,
    prefer_metadata: bool,
) -> Option<v8::Global<v8::Script>> {
    let src = esbuild_cjs_fallback(abs, prefer_metadata)?;
    let wrapped_fb =
        format!("(function (exports, module, require, __filename, __dirname) {{\n{src}\n}})");
    if wrapped_fb == module.wrapped {
        return None; // native wasn't the source (output identical) — no point retrying
    }
    let code_fb = v8::String::new(tc, &wrapped_fb)?;
    v8::Script::compile(tc, code_fb, None).map(|s| v8::Global::new(tc, s))
}

/// Log the native compile failure's exception text when the loader-debug env is on.
fn debug_cjs_compile_failure(
    tc: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope<'_>>>,
    abs: &Path,
) {
    if !loader_debug() {
        return;
    }
    let native_exc = tc
        .exception()
        .map(|e| e.to_rust_string_lossy(tc))
        .unwrap_or_default();
    eprintln!("  cjs compile failed: {} :: {native_exc}", abs.display());
}

/// Put back the `__ttDir` a CJS module load replaced.
fn restore_tt_dir<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    prev_dir: Option<v8::Local<'s, v8::Value>>,
) {
    if let (Some(k), Some(prev)) = (v8::String::new(scope, "__ttDir"), prev_dir) {
        global.set(scope, k.into(), prev);
    }
}

/// A module whose read / transform failed: throw vite's error for it, when there is one.
fn throw_transform_failure(scope: &mut v8::PinScope, abs: &Path) {
    if loader_debug() {
        eprintln!("  cjs read/transform failed: {}", abs.display());
    }
    // a TypeScript / JSX module oxc does not transform: vite's error, the module's compile
    // error (turbo-test#113)
    if let Some(e) = vite_error_value(scope, abs) {
        let g = v8::Global::new(scope, e);
        COMPILE_EXC.with(|c| *c.borrow_mut() = Some((g, abs.to_path_buf())));
        scope.throw_exception(e);
    }
}

/// Re-throw (and record) a module's compile error, when there is one.
fn throw_compile_exc(
    scope: &mut v8::PinScope,
    abs: &Path,
    compile_exc: Option<v8::Global<v8::Value>>,
) {
    if let Some(e) = compile_exc {
        COMPILE_EXC.with(|c| *c.borrow_mut() = Some((e.clone(), abs.to_path_buf())));
        let e = v8::Local::new(scope, &e);
        scope.throw_exception(e);
    }
}

/// The `require` runtime.js `__mkRequire` makes for the CJS module `abs`.
fn cjs_require_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    abs: &Path,
    dir_str: v8::Local<'s, v8::String>,
    filename: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Value>> {
    let mk_key = v8::String::new(scope, "__mkRequire")?;
    let mk = v8::Local::<v8::Function>::try_from(global.get(scope, mk_key.into())?).ok()?;
    let undef = v8::undefined(scope).into();
    let esm_flag = v8::Boolean::new(scope, is_esm_module(abs));
    // pass this module's path as the require importer, so native_require can record the import
    // edge (imported <- importer) for mock-graph invalidation under isolate-reuse.
    mk.call(
        scope,
        undef,
        &[dir_str.into(), esm_flag.into(), filename.into()],
    )
}

/// Run a CJS module's wrapper: whether the body completed, else its exception.
fn call_cjs_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    wrapper: v8::Local<v8::Function>,
    args: &[v8::Local<v8::Value>],
) -> (bool, Option<v8::Global<v8::Value>>) {
    let undef = v8::undefined(scope).into();
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    let r = wrapper.call(tc, undef, args);
    if r.is_none() && tc.has_caught() && loader_debug() {
        let stk = tc
            .stack_trace()
            .map(|s| s.to_rust_string_lossy(tc))
            .unwrap_or_default();
        let exc = tc
            .exception()
            .map(|e| e.to_rust_string_lossy(tc))
            .unwrap_or_default();
        let top = stk.lines().take(4).collect::<Vec<_>>().join(" | ");
        eprintln!("  cjs body threw: {} :: {exc} :: {top}", abs.display());
    }
    let exc = if r.is_none() {
        tc.exception().map(|e| v8::Global::new(tc, e))
    } else {
        None
    };
    (r.is_some(), exc)
}

/// An app decorator file of an emitDecoratorMetadata project (its body may need the metadata).
fn wants_metadata_retry(abs: &Path) -> bool {
    !abs.components().any(|c| c.as_os_str() == "node_modules")
        && decorator_metadata_enabled(abs)
        && std::fs::read_to_string(abs)
            .map(|s| file_has_decorator(&s))
            .unwrap_or(false)
}

/// After a CJS module body ran: drain / discard the vi.mock()s it pushed.
fn settle_module_mocks(scope: &mut v8::PinScope, abs: &Path, drain_mocks: bool) {
    // Apply any vi.mock() this module declared (e.g. a per-test `*.test.setup` helper that
    // mocks/overrides a module). esbuild hoists requires above the body, so by here the only
    // pending mocks are THIS file's — drain them, resolved relative to this file, overriding
    // any earlier (global setup) mock of the same module. (Both real imports and per-file
    // mocks are thus supported: a file-local vi.mock wins for that file's subsequent imports.)
    if RESOLVING_MOCKS.with(|f| f.get()) {
        // Re-entrant load during mock resolution (an async factory's `await import('<self>')`):
        // leave the pending-mock queue alone — the outer resolve+drain owns it.
    } else if drain_mocks {
        let dir_for_mocks = abs.parent().unwrap_or(Path::new("."));
        resolve_pending_mocks(scope);
        drain_pending_mocks(scope, dir_for_mocks, false);
    } else {
        // entry module: its vi.mock()s were already pre-registered by run_entry_mocks (before
        // its imports loaded). Discard the re-pushed pending mocks so we don't double-register
        // with fresh vi.fn()s the already-imported modules aren't bound to.
        let g = scope.get_current_context().global(scope);
        if let Some(k) = v8::String::new(scope, "__pendingMocks") {
            let empty = v8::Array::new(scope, 0);
            g.set(scope, k.into(), empty.into());
        }
    }
}

/// Register a loaded CJS module's final exports (and its synthetic ES module) in the registry.
fn register_cjs_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    abs: &Path,
    final_exports: v8::Local<'s, v8::Value>,
) -> Option<()> {
    mark_external_cjs(scope, global, abs, final_exports);
    let names = object_keys(scope, global, final_exports);
    let exports_global = v8::Global::new(scope, final_exports);
    let synth = make_synthetic(scope, &names)?;
    let hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.real_exports
            .insert(abs.to_path_buf(), exports_global.clone());
        reg.cjs_exports.insert(abs.to_path_buf(), exports_global);
        reg.cjs_export_names.insert(abs.to_path_buf(), names);
        reg.cjs_synth_by_path
            .insert(abs.to_path_buf(), synth_global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
    });
    Some(())
}

/// A package's CommonJS `module.exports` without `__esModule` goes into runtime.js's
/// `__ttExternalCjs`: importing it gives Node's namespace of it (its default the whole
/// `module.exports`, plus `'module.exports'`), as vitest imports an external through Node; an app
/// CommonJS file's own `default` stays its default, as vitest's inlined-module interop gives it
/// (turbo-test#122, #133).
fn mark_external_cjs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    abs: &Path,
    exports: v8::Local<'s, v8::Value>,
) -> Option<()> {
    if !abs.components().any(|c| c.as_os_str() == "node_modules") {
        return None;
    }
    // an object (a primitive `module.exports` has no names to mark) without `__esModule`
    let obj = v8::Local::<v8::Object>::try_from(exports).ok()?;
    let ekey = v8::String::new(scope, "__esModule")?;
    if obj.has(scope, ekey.into()).unwrap_or(true) {
        return None;
    }
    let mkey = v8::String::new(scope, "__ttMarkExternalCjs")?;
    let mark = v8::Local::<v8::Function>::try_from(global.get(scope, mkey.into())?).ok()?;
    // the names Node's cjs-module-lexer finds where they differ from the object's own
    let names = std::fs::read_to_string(abs)
        .ok()
        .and_then(|src| crate::esm_cjs::cjs_literal_export_names(&src));
    let names: v8::Local<v8::Value> = match names {
        Some(names) => {
            let items: Vec<v8::Local<v8::Value>> = names
                .iter()
                .filter_map(|n| v8::String::new(scope, n).map(Into::into))
                .collect();
            v8::Array::new_with_elements(scope, &items).into()
        }
        None => v8::undefined(scope).into(),
    };
    let undefined = v8::undefined(scope).into();
    mark.call(scope, undefined, &[exports, names]);
    Some(())
}

fn object_keys(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    obj: v8::Local<v8::Value>,
) -> Vec<String> {
    let keys_key = v8::String::new(scope, "__keys").unwrap();
    let Some(keys_fn) = global
        .get(scope, keys_key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
    else {
        return vec![];
    };
    let undef = v8::undefined(scope).into();
    let Some(res) = keys_fn.call(scope, undef, &[obj]) else {
        return vec![];
    };
    let Ok(arr) = v8::Local::<v8::Array>::try_from(res) else {
        return vec![];
    };
    (0..arr.length())
        .filter_map(|i| {
            arr.get_index(scope, i)
                .map(|v| v.to_rust_string_lossy(scope))
        })
        .collect()
}

fn make_synthetic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    names: &[String],
) -> Option<v8::Local<'s, v8::Module>> {
    let mut export_names = Vec::with_capacity(names.len() + 1);
    export_names.push(v8::String::new(scope, "default")?);
    for n in names {
        if n != "default" {
            export_names.push(v8::String::new(scope, n)?);
        }
    }
    let mod_name = v8::String::new(scope, "<synthetic>")?;
    Some(v8::Module::create_synthetic_module(
        scope,
        mod_name,
        &export_names,
        synth_eval,
    ))
}

fn synth_eval<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let hash = module.get_identity_hash().get();
    let entry = REGISTRY.with(|r| {
        let reg = r.borrow();
        let path = reg.path_by_hash.get(&hash)?.clone();
        let exports = reg.cjs_exports.get(&path)?.clone();
        // JSON and asset stubs go through the mock table too, but their default is the value
        let is_mock = reg.mocks.contains_key(&path)
            && !is_asset(&path)
            && path.extension().and_then(|e| e.to_str()) != Some("json");
        let names = reg.cjs_export_names.get(&path).cloned().unwrap_or_default();
        let app_cjs =
            !path.components().any(|c| c.as_os_str() == "node_modules") && !path.starts_with("<");
        Some((exports, names, is_mock, app_cjs))
    });
    // A module whose exports were dropped (a reused isolate's reset) fails its importer's
    // evaluation: evaluation steps that return nothing without an exception abort V8
    // (`Check failed: has_exception()`, turbo-test#79).
    let (exports_g, names, is_mock, app_cjs) = entry.or_else(|| throw_none(scope, UNLOADED))?;
    let exports = v8::Local::new(scope, &exports_g);
    let dkey = v8::String::new(scope, "default")?;
    // A mock's (factory result's / automock's) own `default` key IS its default export (vitest,
    // and the module runner's interop). For CommonJS the whole `module.exports` is, except that
    // vitest's interop (`interopModule`) takes `exports.default` of one marked `__esModule` (tsc /
    // babel output) that has a `default` (turbo-test#122).
    // An app CommonJS file (vitest inlines it) has its own `default` as its default either way.
    let esm_marked = |scope: &mut v8::PinScope<'s, '_>, o: v8::Local<v8::Object>| {
        let k = v8::String::new(scope, "__esModule").unwrap();
        (app_cjs || o.has(scope, k.into()).unwrap_or(false))
            && o.has(scope, dkey.into()).unwrap_or(false)
    };
    let default = v8::Local::<v8::Object>::try_from(exports)
        .ok()
        .filter(|o| {
            if is_mock {
                o.has_own_property(scope, dkey.into()).unwrap_or(false)
            } else {
                esm_marked(scope, *o)
            }
        })
        .and_then(|o| o.get(scope, dkey.into()));
    let default = match default {
        Some(d) => d,
        // a vi.mock factory without `default`: its use throws vitest's error (turbo-test#137)
        None if is_mock => mock_default(scope, exports).unwrap_or(exports),
        None => exports,
    };
    module.set_synthetic_module_export(scope, dkey, default)?;
    if let Ok(obj) = v8::Local::<v8::Object>::try_from(exports) {
        set_synthetic_named_exports(scope, module, obj, &names)?;
    }
    Some(v8::undefined(scope).into())
}

/// Bind every non-default name the CJS object actually carries as a synthetic export. A name
/// the object lacks (requested by an importer, missing from a mock factory) stays undefined
/// without being read: an ES import binding is vitest-lenient at link time, and a strict
/// mock's missing-export throw must not fire while linking.
fn set_synthetic_named_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<v8::Module>,
    obj: v8::Local<v8::Object>,
    names: &[String],
) -> Option<()> {
    for n in names {
        if n == "default" {
            continue;
        }
        let k = v8::String::new(scope, n)?;
        if !obj.has(scope, k.into()).unwrap_or(false) {
            continue;
        }
        if let Some(v) = obj.get(scope, k.into()) {
            module.set_synthetic_module_export(scope, k, v)?;
        }
    }
    Some(())
}

/// runtime.js's `__ttMockDefault(exports)`: the default export of a mock whose factory returned
/// no `default` (a stand-in that throws vitest's error when used, turbo-test#137).
fn mock_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exports: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let k = v8::String::new(scope, "__ttMockDefault")?;
    let f = v8::Local::<v8::Function>::try_from(global.get(scope, k.into())?).ok()?;
    let undef = v8::undefined(scope).into();
    f.call(scope, undef, &[exports])
}

// ---- Layer B: lazy module stubs -------------------------------------------
//
// Barrels named in TURBO_LAZY_STUBS (comma-separated bare specifiers, e.g.
// "@mui/icons-material") are NEVER resolved / transformed / evaluated. Each require returns a
// per-isolate Proxy whose every property lazily builds a forwardRef SVG-icon stub component —
// the exact shape @mui/icons-material (and lucide-react etc.) render, mirroring the proven
// setup-optimized.ts mock. Because turbo-test boots a fresh isolate per test file, the real
// icon barrel (~2k re-export modules) would otherwise be re-evaluated for every file that
// touches an icon. The stub replaces that per-file eval with one cheap Proxy. Generic: add a
// specifier to the env list, no code change. Only safe for "display-component" barrels (every
// export is a component) — NOT @mui/material (styled/useTheme/alpha/Box are not components).

/// Configured stub specifiers, parsed once from TURBO_LAZY_STUBS.
fn lazy_stub_specs() -> &'static Vec<String> {
    static SPECS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        std::env::var("TURBO_LAZY_STUBS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// If `spec` is (or is a subpath of) a configured stub barrel, return (cache_key, default_name).
/// For a subpath like "@mui/icons-material/Add" the default export is the `Add` icon; for the
/// package root the default is a generic stub.
fn lazy_stub_match(spec: &str) -> Option<(String, Option<String>)> {
    for e in lazy_stub_specs() {
        if spec == e {
            return Some((spec.to_string(), None));
        }
        if let Some(rest) = spec.strip_prefix(&format!("{e}/")) {
            let base = rest.rsplit('/').next().unwrap_or(rest);
            let base = base.strip_suffix(".js").unwrap_or(base);
            return Some((spec.to_string(), Some(base.to_string())));
        }
    }
    None
}

/// JS source for one stub namespace: a Proxy that lazily mints a forwardRef SVG-icon component
/// per accessed export, caching it. `default_js` is a quoted icon name (subpath default) or "null".
fn lazy_stub_factory_src(default_js: &str) -> String {
    format!(
        r#"(function() {{
  var dir = globalThis.__ttDir || globalThis.__cwd || '.';
  var React = globalThis.__nativeRequire(dir, 'react', false);
  if (React && React.default && typeof React.forwardRef !== 'function') React = React.default;
  var cache = Object.create(null);
  function make(name) {{
    var C = React.forwardRef(function(props, ref) {{
      props = props || {{}};
      var rest = {{}};
      for (var k in props) {{ if (k !== 'data-testid' && k !== 'className' && k !== 'sx') rest[k] = props[k]; }}
      return React.createElement('svg', Object.assign({{
        ref: ref,
        'data-testid': props['data-testid'] || (name + 'Icon'),
        'data-icon': name,
        className: ('MuiSvgIcon-root ' + (props.className || '')).trim(),
        viewBox: '0 0 24 24', width: '1em', height: '1em',
        focusable: 'false', 'aria-hidden': 'true'
      }}, rest));
    }});
    C.displayName = name;
    C.muiName = 'SvgIcon';
    return C;
  }}
  var DEFAULT_NAME = {default_js};
  return new Proxy({{}}, {{
    get: function(t, k) {{
      if (k === '__esModule') return false;
      // not a thenable: `await import(barrel)` resolves its promise WITH this namespace, and an
      // answered `then` made promise resolution call it as a thenable and wait forever
      if (typeof k === 'symbol' || k === 'then') return undefined;
      if (k === '__turboLazyStub') return true;
      if (k === 'default') return cache['__default'] || (cache['__default'] = make(DEFAULT_NAME || 'default'));
      return cache[k] || (cache[k] = make(k));
    }},
    set: function(t, k, v) {{ cache[k] = v; return true; }},
    has: function(t, k) {{ return k !== 'then'; }},
    defineProperty: function() {{ return true; }},
    deleteProperty: function() {{ return true; }},
    getOwnPropertyDescriptor: function() {{ return undefined; }},
    ownKeys: function() {{ return []; }}
  }});
}})()"#,
        default_js = default_js
    )
}

/// Build (or fetch the cached) stub namespace for `cache_key` in the current isolate. Returns
/// None if the factory fails (e.g. react unresolvable) so the caller falls back to a real load.
fn ensure_lazy_stub<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache_key: &str,
    default_name: Option<&str>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(g) = REGISTRY.with(|r| r.borrow().lazy_stub_ns.get(cache_key).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }
    let default_js = match default_name {
        Some(n) => format!("{n:?}"),
        None => "null".to_string(),
    };
    let src = lazy_stub_factory_src(&default_js);
    let code = v8::String::new(scope, &src)?;
    let script = v8::Script::compile(scope, code, None)?;
    let val = script.run(scope)?;
    let g = v8::Global::new(scope, val);
    REGISTRY.with(|r| {
        r.borrow_mut()
            .lazy_stub_ns
            .insert(cache_key.to_string(), g.clone())
    });
    Some(v8::Local::new(scope, &g))
}

/// Register a generic stub for a node_modules module that FAILED to load in turbo-test's shimmed
/// env (a heavy Node-only package — e.g. playwright-core's deep deps — that a test imports but
/// doesn't actually exercise). The stub is a Proxy that is callable, constructable, and returns a
/// fresh stub for any property, so importers don't crash. If the test really uses the dep, its
/// assertions fail loudly — but a mere import no longer takes the whole file down.
fn register_node_stub<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path) -> Option<()> {
    // NOTE: target is a function (callable + constructable). Do NOT override ownKeys/
    // getOwnPropertyDescriptor — a function's `prototype` is a non-configurable own prop, so a
    // [] ownKeys trap violates the Proxy invariant and makes Object.keys(stub) throw.
    let src = "(function(){var make=function(){return new Proxy(function(){},{\
get:function(t,k){if(k==='__esModule')return true;if(k==='default')return make();if(typeof k==='symbol')return t[k];return make();},\
apply:function(){return make();},construct:function(){return make();},has:function(){return true;},\
set:function(){return true;}});};return make();})()";
    let code = v8::String::new(scope, src)?;
    let script = v8::Script::compile(scope, code, None)?;
    let val = script.run(scope)?;
    register_mock_value(scope, abs, val)
}

thread_local! {
    /// Set per file: true when the entry test file lives under an e2e/ dir. e2e HELPER tests
    /// import heavy Node-only packages (e.g. @playwright/test) that can't fully load in the
    /// shimmed env but aren't actually exercised (mocked request contexts). For ONLY those files
    /// we stub a failed node_modules load instead of throwing. Unit tests under src/ keep strict
    /// behavior — stubbing there would mask bugs AND turn a transient turbo-dom parser failure
    /// into a permanent DOM-less stub (mass "document is not defined").
    static ENTRY_LENIENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The test file this thread runs (its mock factories see its scope, mock_scope.rs).
    static TEST_FILE: std::cell::RefCell<PathBuf> = const { std::cell::RefCell::new(PathBuf::new()) };
    /// The current test file's setup files, which get the factory-scope binder too (turbo-test#205).
    static SCOPED_SETUP_FILES: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The test file's top-level bindings its `vi.mock` factories name (mock_scope.rs); the shared
/// `let`s the module runner already routes through `globalThis.__ms` (`mr`) are left out.
fn factory_bindings(entry: &Path, src: &str, mr: bool) -> Vec<crate::mock_scope::Binding> {
    let mocks = extract_mocks(src);
    if mocks.is_empty() {
        return Vec::new();
    }
    let factories: String = mocks
        .iter()
        .map(|(_, f)| f.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let shared = if mr {
        shared_mock_lets(src)
    } else {
        Vec::new()
    };
    crate::mock_scope::bindings(entry, src, &factories)
        .into_iter()
        .filter(|(n, _)| !shared.contains(n))
        .collect()
}

/// `src` of the file `abs` as it loads: the test file gets the binder statement that hands its
/// factories' bindings to the runtime, on a line of its own at the end (so no position moves).
/// An app module's source as vite's transform sees it: `import.meta.glob` calls expanded against
/// the file system (src/import_glob.rs, turbo-test#130, #145), the vitest root (the working
/// directory) the base of `/` patterns, the config's `resolve.alias` (TURBO_ALIAS) its aliases. node_modules are read as they are (vitest externalizes them).
fn read_app_source(file: &Path) -> Option<String> {
    let src = std::fs::read_to_string(file).ok()?;
    if file.components().any(|c| c.as_os_str() == "node_modules") {
        return Some(src);
    }
    let root = std::env::current_dir().ok()?;
    let aliases: Vec<(String, String)> = std::env::var("TURBO_ALIAS")
        .map(|raw| {
            raw.split('\u{1e}')
                .filter_map(|r| r.split_once('\u{1f}'))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Some(crate::import_glob::expand(file, &src, &root, &aliases).unwrap_or(src))
}

fn with_binder(abs: &Path, src: String, mr: bool) -> String {
    // the test file and the setup files: each one's factories close over its own bindings
    // (turbo-test#205)
    if !TEST_FILE.with(|f| *f.borrow() == abs)
        && !SCOPED_SETUP_FILES.with(|f| f.borrow().iter().any(|s| s == abs))
    {
        return src;
    }
    let bindings = factory_bindings(abs, &src, mr);
    if bindings.is_empty() {
        return src;
    }
    format!("{src}\n{}", crate::mock_scope::binder(&bindings, abs))
}

/// Whether to stub a failed node_modules load for the CURRENT file. e2e entries only;
/// disableable with TURBO_NO_STUB_FAIL.
fn stub_failed_deps() -> bool {
    ENTRY_LENIENT.with(|c| c.get()) && std::env::var("TURBO_NO_STUB_FAIL").is_err()
}

fn native_require(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    // importer module kind (ESM vs CJS) selects the export conditions for resolution
    let esm = {
        let a = args.get(2);
        if a.is_undefined() {
            true
        } else {
            a.boolean_value(scope)
        }
    };
    if require_builtin(scope, &mut rv, &spec) {
        return;
    }
    let Some(abs) = resolve_spec_as(&spec, Path::new(&dir), esm) else {
        require_unresolved(scope, &mut rv, &args, &dir, &spec);
        return;
    };
    let abs = query_mock_variant(&spec, abs);
    record_import_edge(scope, &args, &abs);
    if require_special(scope, &mut rv, &abs) {
        return;
    }
    // mocked? A registered mock is served from its cjs_exports below. Its exports stay while it
    // is registered (invalidate_importers never evicts a mocked module, turbo-test#31 item 2);
    // a mocks entry without them would load the real module rather than serve an empty `{}`.
    // (an entry the test put into require.cache is served like a mock)
    let put = CACHE_PUT.with(|c| c.borrow().contains(&abs));
    if require_bypass_mock_for_external(scope, &mut rv, &dir, &abs, put) {
        return;
    }
    let mocked = REGISTRY.with(|r| {
        let reg = r.borrow();
        (put || reg.mocks.contains_key(&abs)) && reg.cjs_exports.contains_key(&abs)
    });
    if !mocked && !require_load_for_kind(scope, &mut rv, &args, &abs, &spec) {
        return;
    }
    // a mock factory that threw: the import fails with its error (vitest)
    if let Some(e) = mock_factory_error(scope, &abs) {
        scope.throw_exception(e);
        return;
    }
    set_cjs_exports(scope, &mut rv, &abs);
}

/// A package's own `require()` sees no `vi.mock`: vitest loads an external through Node, which
/// never consults the mock registry (turbo-test#135). When the current importer is such a
/// node_modules package and `abs` has a registered mock, this serves the REAL module's exports
/// and returns `true`. `put` entries (the test put the key into `require.cache`) opt out and
/// keep the mock.
fn require_bypass_mock_for_external(
    scope: &mut v8::PinScope,
    rv: &mut v8::ReturnValue,
    dir: &str,
    abs: &Path,
    put: bool,
) -> bool {
    if put
        || !dir.contains("/node_modules/")
        || inlined_dep(Path::new(dir))
        || !REGISTRY.with(|r| r.borrow().mocks.contains_key(abs))
    {
        return false;
    }
    if let Some(g) = actual_exports(scope, abs) {
        rv.set(v8::Local::new(scope, &g));
        return true;
    }
    false
}

/// Load the module at `abs` on the CJS or ESM path (deciding by `mr_enabled()` and `detect_kind`).
/// Returns `false` when the load errored (the exception is already set / returned) and
/// `native_require` must stop; `true` when loading succeeded and the caller should serve its
/// exports.
fn require_load_for_kind(
    scope: &mut v8::PinScope,
    rv: &mut v8::ReturnValue,
    args: &v8::FunctionCallbackArguments,
    abs: &Path,
    spec: &str,
) -> bool {
    if mr_enabled() || detect_kind(abs) == Kind::Cjs {
        return require_load_cjs(scope, args, abs, spec);
    }
    require_load_esm(scope, rv, abs, spec);
    false
}

/// Set `rv` to the registered CJS exports of `key`; whether there were any.
fn set_cjs_exports(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, key: &Path) -> bool {
    if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(key).cloned()) {
        let local = v8::Local::new(scope, &g);
        rv.set(local);
        return true;
    }
    false
}

/// `native_require` of a node builtin, the test API or a lazy-stubbed barrel (true if served).
fn require_builtin(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, spec: &str) -> bool {
    // node builtins (fs/path/module/...) -> shimmed module objects
    if is_node_builtin(spec) {
        ensure_node_builtin(scope, spec);
        let key = PathBuf::from(format!("<node:{spec}>"));
        set_cjs_exports(scope, rv, &key);
        return true;
    }
    // vitest / @jest/globals builtin (describe/it/expect/vi/jest from globals) — never load the
    // real package.
    if is_test_api_module(spec) {
        ensure_vitest_builtin(scope);
        set_cjs_exports(scope, rv, &vitest_key());
        return true;
    }
    // Layer B: configured barrel (TURBO_LAZY_STUBS) -> lazy Proxy namespace, never resolved or
    // evaluated. Falls through to the real load if the stub factory fails.
    if let Some((key, def)) = lazy_stub_match(spec) {
        if let Some(ns) = ensure_lazy_stub(scope, &key, def.as_deref()) {
            rv.set(ns);
            return true;
        }
    }
    false
}

/// `native_require` of a specifier that does not resolve: a stub inside node_modules, else
/// vitest's import error / Node's MODULE_NOT_FOUND.
fn require_unresolved(
    scope: &mut v8::PinScope,
    rv: &mut v8::ReturnValue,
    args: &v8::FunctionCallbackArguments,
    dir: &str,
    spec: &str,
) {
    // A node_modules module requiring something unresolvable in this env (a sibling
    // package.json via "..", an optional dep) — return a stub so it doesn't crash. Keyed by a
    // synthetic path so repeats hit the cache.
    if stub_failed_deps() && dir.contains("/node_modules/") {
        let key = PathBuf::from(format!("<unresolved:{dir}:{spec}>"));
        if REGISTRY.with(|r| !r.borrow().cjs_exports.contains_key(&key)) {
            let _ = register_node_stub(scope, &key);
        }
        if set_cjs_exports(scope, rv, &key) {
            return;
        }
    }
    // the module runner's form of a static import: vitest's import error (turbo-test#108)
    let importer = args.get(3);
    let importer_path = (importer.is_string()).then(|| importer.to_rust_string_lossy(scope));
    let imported = importer_path.as_ref().is_some_and(|f| {
        std::fs::read_to_string(f)
            .is_ok_and(|src| crate::parse_error::imports_statically(Path::new(f), &src, spec))
    });
    if let (true, Some(f)) = (imported, &importer_path) {
        let root = std::env::current_dir().unwrap_or_default();
        let (message, code) = import_not_found(spec, f, &root, true);
        throw_with_code(scope, &message, code);
    } else {
        throw_module_not_found(scope, spec, importer);
    }
}

/// Record the import edge (imported <- importer) for mock-graph invalidation under reuse.
fn record_import_edge(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments, abs: &Path) {
    if reuse_isolate_enabled() {
        let importer = args.get(3);
        if !importer.is_undefined() {
            let imp = PathBuf::from(importer.to_rust_string_lossy(scope));
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .import_edges
                    .entry(abs.to_path_buf())
                    .or_default()
                    .insert(imp);
            });
        }
    }
}

/// `native_require` of a module mid-load (circular), an asset / JSON stub or a native addon
/// (true if handled).
fn require_special(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, abs: &Path) -> bool {
    // circular dependency: module abs is mid-execution → return its live (partial) exports.
    if let Some(modg) = REGISTRY.with(|r| r.borrow().loading.get(abs).cloned()) {
        let mod_obj = v8::Local::new(scope, &modg);
        if let Some(ek) = v8::String::new(scope, "exports") {
            if let Some(exp) = mod_obj.get(scope, ek.into()) {
                rv.set(exp);
            }
        }
        return true;
    }
    // CSS/asset/JSON imports -> stub via load_dep (CSS-module proxy / vite's asset string / parsed
    // JSON value)
    if is_asset(abs)
        || crate::vite_asset::is_asset_id(abs)
        || abs.extension().and_then(|e| e.to_str()) == Some("json")
    {
        load_dep(scope, abs);
        set_cjs_exports(scope, rv, abs);
        return true;
    }
    // native addon (.node) -> load via the Node-API host
    if abs.extension().and_then(|e| e.to_str()) == Some("node") {
        require_addon(scope, rv, abs);
        return true;
    }
    false
}

/// `native_require` of a native addon (.node), via the Node-API host.
fn require_addon(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, abs: &Path) {
    let cached = REGISTRY.with(|r| r.borrow().cjs_exports.get(abs).cloned());
    let g = match cached {
        Some(g) => g,
        None => match crate::napi_host::load_addon(scope, abs) {
            Ok(g) => {
                REGISTRY.with(|r| {
                    r.borrow_mut()
                        .cjs_exports
                        .insert(abs.to_path_buf(), g.clone())
                });
                g
            }
            Err(e) => {
                // A native addon that won't dlopen in this env (e.g. fsevents on a CI box, or
                // an arch mismatch) — stub it so an optional require() (chokidar's fsevents)
                // doesn't take the file down.
                if stub_failed_deps() && register_node_stub(scope, abs).is_some() {
                    set_cjs_exports(scope, rv, abs);
                    return;
                }
                throw(scope, &format!("native addon {}: {e}", abs.display()));
                return;
            }
        },
    };
    rv.set(v8::Local::new(scope, &g));
}

/// `native_require`'s CJS load (MR: everything loads as CJS so imports are live + mockable).
/// True when its exports (or a stub) are to be served; false when an error was thrown.
fn require_load_cjs(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    abs: &Path,
    spec: &str,
) -> bool {
    let (loaded, cause) = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let ok = load_cjs(tc, abs, true).is_some();
        let cause = if ok {
            None
        } else {
            tc.exception().map(|e| v8::Global::new(tc, e))
        };
        (ok, cause)
    };
    if loaded {
        return true;
    }
    // A node_modules dep that can't load in the shimmed env -> stub it (don't crash the
    // importer). App modules still hard-fail (a real bug there must surface).
    if abs.to_string_lossy().contains("/node_modules/")
        && stub_failed_deps()
        && register_node_stub(scope, abs).is_some()
    {
        // fall through to return the stub
        return true;
    }
    // The module's own error (body throw, compile error) propagates, as in Node; a
    // load that failed without one (unreadable / untransformable) gets a generic error.
    match cause {
        Some(e) => {
            let e = v8::Local::new(scope, &e);
            let e = as_import_compile_error(scope, args, abs, spec, e);
            scope.throw_exception(e);
        }
        None => throw(scope, &format!("failed to load {spec}")),
    }
    false
}

/// A module that did not compile, imported statically: vitest's import fails with vite's parse
/// error, where a require() fails as Node's (turbo-test#108). Else `e` unchanged.
fn as_import_compile_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments,
    abs: &Path,
    spec: &str,
    mut e: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let compile = COMPILE_EXC.with(|c| c.borrow().clone());
    if let Some((c, _)) = compile.filter(|(_, m)| *m == abs) {
        let importer = args.get(3);
        let imported = importer.is_string() && {
            let f = importer.to_rust_string_lossy(scope);
            std::fs::read_to_string(&f)
                .is_ok_and(|src| crate::parse_error::imports_statically(Path::new(&f), &src, spec))
        };
        if imported && v8::Local::new(scope, &c).strict_equals(e) {
            e = vite_compile_error(scope, abs, e);
            let g = v8::Global::new(scope, e);
            COMPILE_EXC.with(|c| *c.borrow_mut() = Some((g, abs.to_path_buf())));
        }
    }
    e
}

/// require() of ESM (Node 22+ behavior): load + instantiate + evaluate, return the module
/// namespace as the require() result. Handles CJS->ESM requires (e.g. tslib).
fn require_load_esm(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, abs: &Path, spec: &str) {
    let Some(module) = load_graph(scope, abs) else {
        if abs.to_string_lossy().contains("/node_modules/")
            && stub_failed_deps()
            && register_node_stub(scope, abs).is_some()
        {
            set_cjs_exports(scope, rv, abs);
            return;
        }
        throw(scope, &format!("failed to load ESM {spec}"));
        return;
    };
    if module.get_status() == v8::ModuleStatus::Uninstantiated
        && module.instantiate_module(scope, resolve_callback) != Some(true)
    {
        throw(scope, &format!("failed to instantiate ESM {spec}"));
        return;
    }
    if module.get_status() == v8::ModuleStatus::Instantiated {
        let _ = module.evaluate(scope);
        scope.perform_microtask_checkpoint();
    }
    rv.set(module.get_module_namespace());
}

/// __ttRegisterMock(dir, spec, exports): register a mock immediately (vi.mock hoisting). The
/// factory ran in the calling module's scope, so `exports` already closes over the right vars.
/// The mock-registry key for `spec` imported from `dir`: its resolved path, plus the `?query`
/// when it has one. vitest keys mocks by module id, so `vi.mock('./x.js?raw', ...)` is a module
/// of its own and must not replace `./x.js`'s mock (turbo-test#39 item 2) — resolution itself
/// strips the query to find the file.
fn mock_key(spec: &str, dir: &Path) -> Option<PathBuf> {
    let abs = resolve_spec(spec, dir)?;
    Some(match spec.split_once('?') {
        Some((_, query)) => PathBuf::from(format!("{}?{query}", abs.display())),
        None => abs,
    })
}

/// The registry key an import of `spec` (resolved to `abs`) uses: the query-suffixed key when a
/// mock is registered under it (see mock_key), else the module's own path.
fn query_mock_variant(spec: &str, abs: PathBuf) -> PathBuf {
    let Some((_, query)) = spec.split_once('?') else {
        return abs;
    };
    let key = PathBuf::from(format!("{}?{query}", abs.display()));
    // vite's `?raw` / `?url` / asset-with-query ids are modules of their own (turbo-test#148)
    if REGISTRY.with(|r| r.borrow().mocks.contains_key(&key))
        || crate::vite_asset::is_asset_id(&key)
    {
        key
    } else {
        abs
    }
}

fn native_register_mock(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    let exports = args.get(2);
    if exports.is_undefined() {
        return;
    }
    if let Some(key) = mock_key(&spec, Path::new(&dir)) {
        register_mock_value(scope, &key, exports);
    }
}

/// Whether `dir` lies in a package the config's `server.deps.inline` names (TURBO_DEPS_INLINE,
/// `*` = all): vitest runs such a package through its module runner, so its own imports see
/// vi.mock like app code's (turbo-test#216). The owner is the package after the LAST
/// `node_modules` (a scoped name takes two components).
fn inlined_dep(dir: &Path) -> bool {
    let Ok(list) = std::env::var("TURBO_DEPS_INLINE") else {
        return false;
    };
    let s = dir.to_string_lossy();
    let Some(i) = s.rfind("/node_modules/") else {
        return false;
    };
    let mut parts = s[i + "/node_modules/".len()..].split('/');
    let first = parts.next().unwrap_or_default();
    let owner = match (first.starts_with('@'), parts.next()) {
        (true, Some(name)) => format!("{first}/{name}"),
        _ => first.to_string(),
    };
    list.split('\u{1f}').any(|n| n == "*" || n == owner)
}

/// The REAL exports of `abs`, never a registered mock: vi.importActual / importOriginal, and a
/// package's own require() of a mocked module (turbo-test#135).
fn actual_exports(scope: &mut v8::PinScope, abs: &Path) -> Option<v8::Global<v8::Value>> {
    // importActual/importOriginal returns the REAL module, NOT a registered mock (else a factory
    // doing `mockImplementation(actual.fn)` points the spy at itself → infinite recursion).
    // Prefer the genuine exports cached when the real module was loaded (no re-run → no dual
    // instances of singletons like react-query). If never really loaded, load it once with the
    // mock records temporarily removed, then restore them.
    if let Some(g) = REGISTRY.with(|r| r.borrow().real_exports.get(abs).cloned()) {
        return Some(g);
    }
    let saved = REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        if reg.mocks.contains_key(abs) {
            let m = reg.mocks.remove(abs);
            let e = reg.cjs_exports.remove(abs);
            let s = reg.cjs_synth_by_path.remove(abs);
            let n = reg.cjs_export_names.remove(abs);
            Some((m, e, s, n))
        } else {
            None
        }
    });
    let _ = load_cjs(scope, abs, true);
    let real = REGISTRY.with(|r| {
        r.borrow()
            .real_exports
            .get(abs)
            .cloned()
            .or_else(|| r.borrow().cjs_exports.get(abs).cloned())
    });
    if let Some((m, e, s, n)) = saved {
        REGISTRY.with(|r| {
            let mut reg = r.borrow_mut();
            if let Some(m) = m {
                reg.mocks.insert(abs.to_path_buf(), m);
            }
            if let Some(e) = e {
                reg.cjs_exports.insert(abs.to_path_buf(), e);
            }
            if let Some(s) = s {
                reg.cjs_synth_by_path.insert(abs.to_path_buf(), s);
            }
            if let Some(n) = n {
                reg.cjs_export_names.insert(abs.to_path_buf(), n);
            }
        });
    }
    real
}

/// __ttImportActual(dir, spec): load the REAL module (for vi.importActual / importOriginal in
/// partial mocks like `{ ...await vi.importActual('@tanstack/react-query'), useQueryClient }`)
/// and return its exports object so the factory can spread the real named exports.
fn native_import_actual(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    if is_node_builtin(&spec) {
        ensure_node_builtin(scope, &spec);
        let key = PathBuf::from(format!("<node:{spec}>"));
        if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&key).cloned()) {
            rv.set(v8::Local::new(scope, &g));
        }
        return;
    }
    let Some(abs) = resolve_spec(&spec, Path::new(&dir)) else {
        return;
    };
    if let Some(g) = actual_exports(scope, &abs) {
        rv.set(v8::Local::new(scope, &g));
    }
}

/// __ttResolveSpec(dir, spec): the mock-registry key of `spec` from `dir` (its absolute path,
/// `?query` kept; undefined when it does not resolve). runtime.js keys the entry prepass's
/// once-only factories by it.
fn native_resolve_spec(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    if let Some(abs) = mock_key(&spec, Path::new(&dir)) {
        if let Some(v) = v8::String::new(scope, &abs.to_string_lossy()) {
            rv.set(v.into());
        }
    }
}

/// __ttUnmock(dir, spec): vi.unmock()/doUnmock() — remove a registered mock + its loaded module
/// so the next import loads the REAL module.
fn native_unmock(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    if let Some(abs) = mock_key(&spec, Path::new(&dir)) {
        REGISTRY.with(|r| {
            let mut reg = r.borrow_mut();
            reg.mocks.remove(&abs);
            reg.cjs_exports.remove(&abs);
            reg.real_exports.remove(&abs);
            reg.cjs_synth_by_path.remove(&abs);
            reg.cjs_export_names.remove(&abs);
            reg.esm_by_path.remove(&abs);
        });
    }
}

/// __ttResetModules(): vi.resetModules() — drop loaded APP modules so the next import re-runs
/// them (with any vi.doMock now in effect). Keeps node_modules (react/singletons) and the mock
/// registry, so re-imports re-apply mocks + reuse the one shared react instance.
fn native_reset_modules(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let app: Vec<PathBuf> = reg
            .cjs_synth_by_path
            .keys()
            .chain(reg.cjs_exports.keys())
            .filter(|p| !p.to_string_lossy().contains("node_modules") && !p.starts_with("<"))
            .cloned()
            .collect();
        for p in app {
            // keep mocked paths registered (re-import re-applies the mock); drop only real loads.
            if reg.mocks.contains_key(&p) {
                continue;
            }
            reg.cjs_exports.remove(&p);
            reg.real_exports.remove(&p);
            reg.cjs_synth_by_path.remove(&p);
            reg.cjs_export_names.remove(&p);
            reg.esm_by_path.remove(&p);
        }
    });
}

thread_local! {
    /// The CJS modules executing on this thread, outermost first: Node's require stack.
    static REQUIRE_STACK: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    /// The paths whose require.cache entry the test put there: require() serves them as is.
    static CACHE_PUT: RefCell<std::collections::HashSet<PathBuf>> = RefCell::default();
}

/// Throw Node's MODULE_NOT_FOUND error for `spec` required by `importer` (runtime.js
/// `__ttModuleNotFound` builds it: the message names the require stack).
fn throw_module_not_found(scope: &mut v8::PinScope, spec: &str, importer: v8::Local<v8::Value>) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__ttModuleNotFound").unwrap();
    let spec = v8::String::new(scope, spec).unwrap();
    let make = global.get(scope, key.into()).unwrap();
    let make = v8::Local::<v8::Function>::try_from(make).unwrap();
    let recv = v8::undefined(scope).into();
    if let Some(err) = make.call(scope, recv, &[spec.into(), importer]) {
        scope.throw_exception(err);
    }
}

/// __ttRequireStack() -> the paths of the CJS modules executing, innermost first.
fn native_require_stack(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let paths: Vec<v8::Local<v8::Value>> = REQUIRE_STACK.with(|st| {
        st.borrow()
            .iter()
            .rev()
            .map(|p| v8::String::new(scope, &p.to_string_lossy()).unwrap().into())
            .collect()
    });
    rv.set(v8::Array::new_with_elements(scope, &paths).into());
}

/// __ttRequireResolve(dir, spec, esm) -> require.resolve(spec) from `dir`: the resolved path
/// (a builtin or a test API module is its own name), undefined when nothing resolves.
fn native_require_resolve(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dir = args.get(0).to_rust_string_lossy(scope);
    let spec = args.get(1).to_rust_string_lossy(scope);
    let esm = args.get(2).boolean_value(scope);
    let resolved = if is_node_builtin(&spec) {
        Some(spec.clone())
    } else {
        resolve_spec_as(&spec, Path::new(&dir), esm)
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| is_test_api_module(&spec).then(|| spec.clone()))
    };
    if let Some(r) = resolved {
        rv.set(v8::String::new(scope, &r).unwrap().into());
    }
}

/// require.cache, as Node's Module._cache: the loaded modules by path. `__ttCache(op, path,
/// value)`: "keys" -> the paths of the modules loaded from files; "get" -> a module's exports
/// (undefined when it is not loaded); "delete" -> drop it, so the next require() loads it
/// again (a mock stays registered); "set" -> require() of the path gives `value`.
fn native_require_cache(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let op = args.get(0).to_rust_string_lossy(scope);
    let path = PathBuf::from(args.get(1).to_rust_string_lossy(scope));
    match op.as_str() {
        "keys" => {
            let keys: Vec<PathBuf> = REGISTRY.with(|r| {
                let reg = r.borrow();
                let mut keys: Vec<PathBuf> = reg
                    .cjs_exports
                    .keys()
                    .filter(|p| p.is_absolute())
                    .cloned()
                    .collect();
                keys.sort();
                keys
            });
            let keys: Vec<v8::Local<v8::Value>> = keys
                .iter()
                .map(|p| v8::String::new(scope, &p.to_string_lossy()).unwrap().into())
                .collect();
            rv.set(v8::Array::new_with_elements(scope, &keys).into());
        }
        "get" => {
            if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&path).cloned()) {
                rv.set(v8::Local::new(scope, &g));
            }
        }
        "delete" => REGISTRY.with(|r| {
            CACHE_PUT.with(|c| c.borrow_mut().remove(&path));
            let mut reg = r.borrow_mut();
            reg.cjs_exports.remove(&path);
            reg.real_exports.remove(&path);
            reg.cjs_synth_by_path.remove(&path);
            reg.cjs_export_names.remove(&path);
            reg.esm_by_path.remove(&path);
        }),
        _ => {
            let value = v8::Global::new(scope, args.get(2));
            REGISTRY.with(|r| r.borrow_mut().cjs_exports.insert(path.clone(), value));
            CACHE_PUT.with(|c| c.borrow_mut().insert(path));
        }
    }
}

const UNLOADED: &str = "turbo-test: an imported module was unloaded before it was evaluated";

/// `throw` for an `Option`-returning V8 callback: throws `msg`, then None.
fn throw_none<T>(scope: &mut v8::PinScope, msg: &str) -> Option<T> {
    throw(scope, msg);
    None
}

fn throw(scope: &mut v8::PinScope, msg: &str) {
    let m = v8::String::new(scope, msg).unwrap();
    let exc = v8::Exception::error(scope, m);
    scope.throw_exception(exc);
}

/// Throw an `Error` with `msg` and a `code` property.
fn throw_with_code(scope: &mut v8::PinScope, msg: &str, code: &str) -> Option<()> {
    let m = v8::String::new(scope, msg)?;
    let k = v8::String::new(scope, "code")?;
    let c = v8::String::new(scope, code)?;
    let exc = v8::Exception::error(scope, m);
    exc.cast::<v8::Object>().set(scope, k.into(), c.into());
    scope.throw_exception(exc);
    Some(())
}

/// The error a module `abs` that V8 did not compile (`exc`) fails with: vite's for a module that
/// does not parse or transform (vite_error_value; turbo-test#108, #113), else V8's.
fn vite_compile_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    exc: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    vite_error_value(scope, abs).unwrap_or(exc)
}

/// vite's error for the module `abs` when it does not parse (JavaScript) or transform
/// (TypeScript / JSX), crate::parse_error, as vitest's module runner throws it: an Error with
/// vite's name and message whose `__vitest_rollup_error__` holds what vite's plugin container
/// adds (plugin, id, loc, frame — vitest's reporters print them). None for a module oxc takes
/// and for a dependency in node_modules (vitest leaves those to Node).
fn vite_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Value>> {
    if abs.components().any(|c| c.as_os_str() == "node_modules") {
        return None;
    }
    let root = std::env::current_dir().unwrap_or_default();
    let src = std::fs::read_to_string(abs).ok()?;
    let dir = abs.parent()?;
    let resolve = |spec: &str| resolve_spec(spec, dir).filter(|p| p.is_file());
    let vite = crate::parse_error::vite_error(abs, &src, &root, &resolve)?;
    let message = v8::String::new(scope, &vite.message)?;
    let e = v8::Exception::error(scope, message);
    let o = e.cast::<v8::Object>();
    if vite.name != "Error" {
        let name = v8::String::new(scope, vite.name)?;
        set_prop(scope, o, "name", name.into());
    }
    let details = build_vite_rollup_details(scope, &vite)?;
    set_prop(scope, o, "__vitest_rollup_error__", details.into());
    Some(e)
}

/// Set `o[k] = v`, skipping silently when the key fails to allocate. The `vite_error_value`
/// helpers reuse this instead of unwrapping the key allocation at every assignment.
fn set_prop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    o: v8::Local<'s, v8::Object>,
    k: &str,
    v: v8::Local<'s, v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, k) {
        o.set(scope, k.into(), v);
    }
}

/// Build the `__vitest_rollup_error__` details object vite's plugin container attaches to the
/// error: `plugin` (when set), `id`, `loc: {line, column}` (when set), and `frame` (when set).
fn build_vite_rollup_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    vite: &crate::parse_error::ViteError,
) -> Option<v8::Local<'s, v8::Object>> {
    let details = v8::Object::new(scope);
    if let Some(plugin) = vite.plugin {
        let v = v8::String::new(scope, plugin)?;
        set_prop(scope, details, "plugin", v.into());
    }
    let id = v8::String::new(scope, &vite.id)?;
    set_prop(scope, details, "id", id.into());
    if let Some((line, column)) = vite.loc {
        let loc = v8::Object::new(scope);
        let line = v8::Number::new(scope, line as f64);
        set_prop(scope, loc, "line", line.into());
        let column = v8::Number::new(scope, column as f64);
        set_prop(scope, loc, "column", column.into());
        set_prop(scope, details, "loc", loc.into());
    }
    if let Some(frame) = &vite.frame {
        let v = v8::String::new(scope, frame)?;
        set_prop(scope, details, "frame", v.into());
    }
    Some(details)
}

/// TURBO_CJS_DEBUG: print the loaders' diagnostics (failed reads, transforms, compiles, module
/// bodies that threw) to stderr; TURBO_NATIVE_CJS_STRICT (the native emitter's conformity
/// measurement) prints them too. Otherwise stderr carries none of them (vitest prints none,
/// turbo-test#113).
fn loader_debug() -> bool {
    std::env::var("TURBO_CJS_DEBUG").is_ok() || crate::esm_cjs::strict()
}

// ---- ESM graph ------------------------------------------------------------

/// An app module importing the default of a mock whose `vi.mock` factory returned none: its
/// source with every read of that binding guarded (mock_read.rs), so each throws vitest's error
/// as on vitest's own module runner (turbo-test#151). `None` for every other module. A package
/// is left alone: vitest loads it through Node, which never sees a mock (turbo-test#135).
fn guard_mock_default_reads(scope: &mut v8::PinScope, abs: &Path, src: &str) -> Option<String> {
    let external = abs.components().any(|c| c.as_os_str() == "node_modules")
        && !abs.parent().is_some_and(inlined_dep);
    if external || !src.contains("import") {
        return None;
    }
    let mocks: Vec<(PathBuf, v8::Global<v8::Value>)> = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.mocks
            .keys()
            .filter_map(|p| Some((p.clone(), reg.cjs_exports.get(p)?.clone())))
            .collect()
    });
    if mocks.is_empty() {
        return None;
    }
    let global = scope.get_current_context().global(scope);
    let k = v8::String::new(scope, "__ttMockGuarded")?;
    let f = v8::Local::<v8::Function>::try_from(global.get(scope, k.into())?).ok()?;
    let undef: v8::Local<v8::Value> = v8::undefined(scope).into();
    let guarded: std::collections::HashSet<PathBuf> = mocks
        .into_iter()
        .filter(|(_, g)| {
            let exports = v8::Local::new(scope, g);
            f.call(scope, undef, &[exports])
                .is_some_and(|v| v.boolean_value(scope))
        })
        .map(|(p, _)| p)
        .collect();
    if guarded.is_empty() {
        return None;
    }
    let dir = abs.parent()?;
    crate::mock_read::guard_default_reads(src, &|spec| {
        mock_key(spec, dir).is_some_and(|p| guarded.contains(&p))
    })
}

/// An app module's source rewritten so `vi.spyOn` on its namespace reaches its importers (the
/// column shifts recorded for locations); a node_modules module's unchanged.
fn spyable_app_module(abs: &Path, raw: String) -> String {
    if abs.components().any(|c| c.as_os_str() == "node_modules") {
        return raw;
    }
    match crate::esm_spy::spyable(abs, &raw) {
        Some((code, shifts)) => {
            crate::location::set_shifts(abs, shifts);
            code
        }
        None => {
            crate::location::set_shifts(abs, Vec::new());
            raw
        }
    }
}

fn load_graph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    if let Some(g) = REGISTRY.with(|r| r.borrow().esm_by_path.get(abs).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }
    let Some(raw) = read_transformed(abs, false, false) else {
        // a TypeScript / JSX module oxc does not transform: vite's error (turbo-test#113)
        if let Some(e) = vite_error_value(scope, abs) {
            scope.throw_exception(e);
        }
        return None;
    };
    // A generated bundle in the cache dir has no node_modules beside it; resolve its externalized
    // bare imports relative to the recorded source dir (set by esbuild_bundle_full).
    let dir = REGISTRY
        .with(|r| r.borrow().bundle_src_dir.get(abs).cloned())
        .or_else(|| abs.parent().map(|p| p.to_path_buf()))
        .unwrap();
    let raw = graph_source_rewrites(scope, abs, &dir, raw);
    let code = v8::String::new(scope, &raw)?;
    let name = v8::String::new(scope, &abs.to_string_lossy())?;
    let origin = module_origin(scope, name);
    let mut source = v8::script_compiler::Source::new(code, Some(&origin));
    let module = compile_graph_module(scope, abs, &mut source)?;

    let hash = module.get_identity_hash().get();
    let global = v8::Global::new(scope, module);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.esm_by_path.insert(abs.to_path_buf(), global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
    });

    load_module_requests(scope, abs, &dir, module)?;
    Some(module)
}

/// Run `load_graph`'s source rewrites on an app module's raw text: the mock-default-reads guard
/// (turbo-test#151), the `vi.spyOn` namespace rewrite (turbo-test#131/#146), the CJS graph
/// interop rewrite, and the ESM `__dirname` / `__filename` injection. Each pass leaves the source
/// unchanged when it does not apply.
fn graph_source_rewrites(scope: &mut v8::PinScope, abs: &Path, dir: &Path, raw: String) -> String {
    // reads of a no-default mock's default throw (turbo-test#151); first, so the spy rewrite
    // below reads the guarded value through the overrides
    let raw = guard_mock_default_reads(scope, abs, &raw).unwrap_or(raw);
    // vi.spyOn on an app module's namespace reaches its importers (turbo-test#131, #146); test
    // locations / failure frames map V8's columns back through the in-line edits
    let raw = spyable_app_module(abs, raw);
    let raw = graph_interop(abs, dir, &raw).unwrap_or(raw);
    // CJS gets __dirname/__filename via the wrapper params; a real ES module has neither in scope.
    inject_dirname_esm(abs, &raw).unwrap_or(raw)
}

/// Compile the prepared source under a `TryCatch` and, on V8 failure, re-throw either vite's
/// error (turbo-test#113) or V8's own compile exception.
fn compile_graph_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    source: &mut v8::script_compiler::Source,
) -> Option<v8::Local<'s, v8::Module>> {
    let compiled = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        match v8::script_compiler::compile_module(tc, source) {
            Some(m) => Ok(v8::Global::new(tc, m)),
            None => Err(tc.exception().map(|e| v8::Global::new(tc, e))),
        }
    };
    match compiled {
        Ok(m) => Some(v8::Local::new(scope, &m)),
        Err(exc) => {
            if let Some(exc) = exc {
                let exc = v8::Local::new(scope, &exc);
                let exc = vite_compile_error(scope, abs, exc);
                scope.throw_exception(exc);
            }
            None
        }
    }
}

/// Walk the module's static imports and load each one (or its builtin shim). A static import that
/// does not resolve throws vitest's error (turbo-test#108).
fn load_module_requests(
    scope: &mut v8::PinScope,
    abs: &Path,
    dir: &Path,
    module: v8::Local<v8::Module>,
) -> Option<()> {
    let requests = module.get_module_requests();
    for i in 0..requests.length() {
        let req =
            v8::Local::<v8::ModuleRequest>::try_from(requests.get(scope, i).unwrap()).unwrap();
        let spec = req.get_specifier().to_rust_string_lossy(scope);
        if is_test_api_module(&spec) {
            ensure_vitest_builtin(scope)?;
            continue;
        }
        if is_node_builtin(&spec) {
            ensure_node_builtin(scope, &spec)?;
            continue;
        }
        let Some(dep) = resolve_spec(&spec, dir) else {
            let root = std::env::current_dir().unwrap_or_default();
            let (message, code) = import_not_found(&spec, &abs.to_string_lossy(), &root, true);
            throw_with_code(scope, &message, code);
            return None;
        };
        load_dep(scope, &dep)?;
    }
    Some(())
}

/// `esm_cjs::graph_interop_rewrite` of a graph module's source: its requests of a CommonJS module
/// (not a builtin, asset or JSON) read vitest's interop namespace, or Node's in a package's ES
/// module Node runs as one (turbo-test#133).
fn graph_interop(abs: &Path, dir: &Path, src: &str) -> Option<String> {
    let in_package = abs.components().any(|c| c.as_os_str() == "node_modules");
    let node = in_package && is_esm_module(abs);
    let target = |spec: &str| {
        use crate::esm_cjs::Target;
        if is_test_api_module(spec) || is_node_builtin(spec) || lazy_stub_match(spec).is_some() {
            return Target::Other;
        }
        match resolve_spec(spec, dir) {
            Some(p) if is_asset(&p) || p.extension().and_then(|e| e.to_str()) == Some("json") => {
                Target::Other
            }
            Some(p) if detect_kind(&p) == Kind::Cjs => Target::Cjs,
            Some(p) if !p.components().any(|c| c.as_os_str() == "node_modules") => Target::AppEsm,
            _ => Target::Other,
        }
    };
    crate::esm_cjs::graph_interop_rewrite(src, &target, node)
}

/// An app ES module's namespace as vitest lists it: its names in source order, tagged `Module`
/// (runtime.js `__ttOrderedNs`, turbo-test#141).
fn ordered_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    ns: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let order = read_transformed(abs, false, false)
        .and_then(|src| crate::esm_cjs::export_order(&src))
        .unwrap_or_default();
    let items: Vec<v8::Local<v8::Value>> = order
        .iter()
        .filter_map(|n| v8::String::new(scope, n).map(Into::into))
        .collect();
    let order = v8::Array::new_with_elements(scope, &items);
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__ttOrderedNs")?;
    let f = v8::Local::<v8::Function>::try_from(global.get(scope, key.into())?).ok()?;
    let undefined = v8::undefined(scope).into();
    f.call(scope, undefined, &[ns, order.into()])
}

/// The `?__tt_*` request kind of `spec`.
fn variant_of(spec: &str) -> Option<Variant> {
    use crate::esm_cjs::{NODE_QUERY, NS_QUERY, STAR_QUERY};
    if spec.ends_with(NS_QUERY) {
        Some(Variant::Ns)
    } else if spec.ends_with(STAR_QUERY) {
        Some(Variant::Star)
    } else if spec.ends_with(NODE_QUERY) {
        Some(Variant::Node)
    } else {
        None
    }
}

/// runtime.js's view of the CommonJS exports `exports` for `kind`: vitest's interop namespace
/// (`__ttToESM`), or Node's (`__ttToESMDep(m, 1)`).
fn variant_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exports: v8::Local<'s, v8::Value>,
    kind: Variant,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let (name, args): (&str, Vec<v8::Local<v8::Value>>) = if kind == Variant::Node {
        (
            "__ttToESMDep",
            vec![exports, v8::Boolean::new(scope, true).into()],
        )
    } else {
        ("__ttToESM", vec![exports])
    };
    let key = v8::String::new(scope, name)?;
    let f = v8::Local::<v8::Function>::try_from(global.get(scope, key.into())?).ok()?;
    let undefined = v8::undefined(scope).into();
    f.call(scope, undefined, &args)
}

/// A synthetic module of the CommonJS module `abs` (loaded: its exports registered) for a
/// `kind` request; `None` when `abs` has no CommonJS exports.
fn variant_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    kind: Variant,
) -> Option<v8::Local<'s, v8::Module>> {
    let (exports, esm) = REGISTRY.with(|r| {
        let reg = r.borrow();
        // an ES module (not mocked): its namespace, read once it is evaluated
        match reg.esm_by_path.get(abs) {
            Some(_) if kind == Variant::Ns && !reg.mocks.contains_key(abs) => (None, true),
            _ => (reg.cjs_exports.get(abs).cloned(), false),
        }
    });
    // (no CommonJS exports and no ES module: not loaded yet, None)
    let exports = match exports {
        Some(e) => v8::Local::new(scope, &e),
        None => esm.then(|| v8::undefined(scope).into())?,
    };
    let names = match kind {
        Variant::Ns => vec!["default".to_string()],
        Variant::Star | Variant::Node => {
            let view = variant_view(scope, exports, kind)?;
            let global = scope.get_current_context().global(scope);
            let mut names = object_keys(scope, global, view);
            // (Node's view names `default` for every CommonJS module, turbo-test#193)
            if kind == Variant::Star {
                names.retain(|n| n != "default");
            }
            names
        }
    };
    let mut export_names = Vec::with_capacity(names.len());
    for n in &names {
        export_names.push(v8::String::new(scope, n)?);
    }
    let mod_name = v8::String::new(scope, "<synthetic>")?;
    let module = v8::Module::create_synthetic_module(scope, mod_name, &export_names, variant_eval);
    let hash = module.get_identity_hash().get();
    REGISTRY.with(|r| {
        r.borrow_mut()
            .variant_by_hash
            .insert(hash, (abs.to_path_buf(), kind, names))
    });
    Some(module)
}

fn variant_eval<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let hash = module.get_identity_hash().get();
    let entry = REGISTRY.with(|r| {
        let reg = r.borrow();
        let (path, kind, names) = reg.variant_by_hash.get(&hash)?.clone();
        if kind == Variant::Ns && !reg.mocks.contains_key(&path) {
            if let Some(m) = reg.esm_by_path.get(&path) {
                return Some((Err((m.clone(), path)), kind, names));
            }
        }
        let exports = reg.cjs_exports.get(&path)?.clone();
        Some((Ok(exports), kind, names))
    });
    // a module whose exports were dropped (a reused isolate's reset) fails its importer, as in
    // synth_eval
    let (exports, kind, names) = entry.or_else(|| throw_none(scope, UNLOADED))?;
    let view = variant_eval_view(scope, exports, kind)?;
    if kind == Variant::Ns {
        let k = v8::String::new(scope, "default")?;
        module.set_synthetic_module_export(scope, k, view)?;
        return Some(v8::undefined(scope).into());
    }
    copy_variant_named_exports(scope, module, view, &names);
    Some(v8::undefined(scope).into())
}

/// Build the `view` that `variant_eval`'s named-export copy reads from: either CJS exports as
/// seen through vitest's / Node's interop (`variant_view`), or an ES module's namespace in source
/// order (`ordered_namespace`).
fn variant_eval_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exports: Result<v8::Global<v8::Value>, (v8::Global<v8::Module>, PathBuf)>,
    kind: Variant,
) -> Option<v8::Local<'s, v8::Value>> {
    match exports {
        Ok(exports) => {
            let exports = v8::Local::new(scope, &exports);
            variant_view(scope, exports, kind)
        }
        Err((module, path)) => {
            let module = v8::Local::new(scope, &module);
            let ns = module.get_module_namespace();
            ordered_namespace(scope, &path, ns)
        }
    }
}

/// Copy each name in `names` from the `view` object to the synthetic module's own exports. A
/// name the view lacks stays undefined without being read (see `synth_eval`).
fn copy_variant_named_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<v8::Module>,
    view: v8::Local<'s, v8::Value>,
    names: &[String],
) {
    let Ok(obj) = v8::Local::<v8::Object>::try_from(view) else {
        return;
    };
    for n in names {
        let Some(k) = v8::String::new(scope, n) else {
            return;
        };
        if obj.has(scope, k.into()).unwrap_or(false) {
            if let Some(v) = obj.get(scope, k.into()) {
                if module.set_synthetic_module_export(scope, k, v).is_none() {
                    return;
                }
            }
        }
    }
}

thread_local! {
    /// The last compile error a CommonJS load threw (the module runner's form of an import) and
    /// the module it is of.
    static COMPILE_EXC: RefCell<Option<(v8::Global<v8::Value>, PathBuf)>> =
        const { RefCell::new(None) };
}

/// Load a file's entry module graph under a TryCatch: the error is the exception the load
/// threw — a module of the graph that failed to compile gives V8's `SyntaxError: …`, as vitest
/// reports it, where V8 printed an uncaught compile error to stdout (turbo-test#99) — else
/// `graph load failed` (a specifier that does not resolve).
fn load_entry_graph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Result<v8::Local<'s, v8::Module>, String> {
    let module = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        match load_graph(tc, abs) {
            Some(m) => v8::Global::new(tc, m),
            None => {
                let exc = tc.exception();
                note_load_error(tc, exc);
                // the module the exception was thrown in (for a compile error, the one that
                // did not compile)
                let module = tc
                    .message()
                    .and_then(|m| m.get_script_resource_name(tc))
                    .map(|n| PathBuf::from(n.to_rust_string_lossy(tc)));
                LOAD_ERROR.with(|e| {
                    if let Some(info) = e.borrow_mut().as_mut() {
                        info.source = module;
                    }
                });
                return Err(caught_message(tc, "graph load failed")
                    .unwrap_or_else(|| "graph load failed".to_string()));
            }
        }
    };
    Ok(v8::Local::new(scope, &module))
}

/// Registry key for the built-in test-API module shim (named import surface).
/// One shim backs both `vitest` and `@jest/globals` — the exported names are a superset
/// (describe/it/test/expect/vi/jest/hooks/assert), all sourced from our runtime globals.
fn vitest_key() -> PathBuf {
    PathBuf::from("<builtin:vitest>")
}

/// Is `spec` a bare test-framework module we satisfy from globals rather than node_modules?
/// Covers vitest (`import … from 'vitest'`) and jest (`import { jest } from '@jest/globals'`).
fn is_test_api_module(spec: &str) -> bool {
    spec == "vitest" || spec == "@jest/globals"
}

/// Expose our runtime globals as a `vitest` module so real suites that do
/// `import { describe, it, expect, vi } from 'vitest'` resolve. Idempotent.
/// (Real @vitest/* is baked into the snapshot at M3; this is the M1 bridge.)
fn ensure_vitest_builtin<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<()> {
    let key = vitest_key();
    if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(&key)) {
        return Some(());
    }
    let global = scope.get_current_context().global(scope);
    let obj = v8::Object::new(scope);
    // export name -> the global holding it (chai's assert / chai / should are kept on hidden
    // globals, turbo-test#160)
    for (n, g) in [
        ("describe", "describe"),
        ("it", "it"),
        ("test", "test"),
        ("expect", "expect"),
        ("expectTypeOf", "expectTypeOf"),
        ("assertType", "assertType"),
        ("vi", "vi"),
        ("jest", "jest"),
        ("beforeEach", "beforeEach"),
        ("afterEach", "afterEach"),
        ("beforeAll", "beforeAll"),
        ("afterAll", "afterAll"),
        ("onTestFinished", "onTestFinished"),
        ("onTestFailed", "onTestFailed"),
        ("assert", "__ttVitestAssert"),
        ("chai", "__ttVitestChai"),
        ("should", "__ttVitestShould"),
    ] {
        let k = v8::String::new(scope, n)?;
        let gk = v8::String::new(scope, g)?;
        if let Some(v) = global.get(scope, gk.into()) {
            if !v.is_undefined() {
                obj.set(scope, k.into(), v);
            }
        }
    }
    let exports_val: v8::Local<v8::Value> = obj.into();
    let names = object_keys(scope, global, exports_val);
    let exports_global = v8::Global::new(scope, exports_val);
    let synth = make_synthetic(scope, &names)?;
    let hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.cjs_exports.insert(key.clone(), exports_global);
        reg.cjs_export_names.insert(key.clone(), names);
        reg.cjs_synth_by_path.insert(key.clone(), synth_global);
        reg.path_by_hash.insert(hash, key);
    });
    Some(())
}

/// Load a dependency by path, honoring mocks and kind.
fn load_dep<'s>(scope: &mut v8::PinScope<'s, '_>, dep: &Path) -> Option<()> {
    // a registered mock (its synthetic module is what importers link against; see native_require
    // for why a mocked path always has one)
    let mocked = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.mocks.contains_key(dep) && reg.cjs_synth_by_path.contains_key(dep)
    });
    if mocked {
        return Some(());
    }
    // vite's asset imports (turbo-test#148): `default` is the `?raw` text / the URL / the inline
    // svg; any other name reads as itself, as the asset stub below
    let root = std::env::current_dir().unwrap_or_default();
    if let Some(v) = crate::vite_asset::value(dep, &root) {
        let v = serde_json::to_string(&v).ok()?;
        return register_mock(
            scope,
            dep,
            &format!("() => new Proxy({{ default: {v} }}, {{ get: (t, p) => p in t ? t[p] : typeof p === 'string' ? p : undefined }})"),
        );
    }
    // CSS/asset imports -> stub (CSS-module proxy: any class name -> its own string). Assets and
    // JSON are registered via register_mock, i.e. recorded in `mocks`, so an already-loaded one
    // is served by the mocked branch above; reaching here means it is not loaded yet.
    // (every other asset type is one of vite's, served above)
    if matches!(
        dep.extension().and_then(|e| e.to_str()),
        Some("css" | "scss" | "sass" | "less" | "styl")
    ) {
        // Stylesheets: the proxy is also its own `default` and is not `__esModule`, so the
        // module-runner's CJS interop (`__toESM(require(..)).default`) reaches it too and
        // `import s from './s.module.css'` gives `s.foo === 'foo'` on both paths.
        return register_mock(
            scope,
            dep,
            "() => { const s = new Proxy({}, { get: (t, p) => p === 'default' ? s : typeof p === 'string' && p !== '__esModule' ? p : undefined }); return s; }",
        );
    }
    // JSON imports -> module with default = parsed value + named keys
    if dep.extension().and_then(|e| e.to_str()) == Some("json") {
        let txt = std::fs::read_to_string(dep).ok()?;
        return register_mock(scope, dep, &format!("() => ({txt})"));
    }
    match detect_kind(dep) {
        Kind::Esm => load_graph(scope, dep).map(|_| ()),
        Kind::Cjs => load_cjs(scope, dep, true),
    }
}

fn resolve_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _attrs: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let spec = specifier.to_rust_string_lossy(scope);
    let lookup = resolve_spec_lookup(scope, referrer, &spec);
    let abs = query_mock_variant(&spec, lookup?);
    // a mock factory that threw: linking the importer fails with its error (vitest)
    if let Some(e) = mock_factory_error(scope, &abs) {
        scope.throw_exception(e);
        return None;
    }
    // an external package's own ES import of a mocked ES module: the real one (turbo-test#218)
    if let Some(m) = real_esm_for(scope, referrer, &abs) {
        return Some(m);
    }
    // a CommonJS module's namespace view (graph_interop)
    if let Some(kind) = variant_of(&spec) {
        if let Some(m) = variant_module(scope, &abs, kind) {
            return Some(m);
        }
    }
    if let Some(m) = loaded_module(scope, &abs) {
        return Some(m);
    }
    // Externalized package not loaded yet (a setup bundle's `import 'react'` etc. under
    // --packages=external): lazy-load it now via the shared loaders so the bundle binds to the
    // ONE instance every other importer gets, instead of a copy baked into the bundle.
    if mr_enabled() && esbuild_transform_cjs(&abs, false).is_some() {
        load_cjs(scope, &abs, true);
    } else {
        load_dep(scope, &abs);
    }
    loaded_module(scope, &abs)
}

/// The registry key for a `resolve_callback` specifier: vitest / node builtins resolve to
/// their `<...>` key (also ensuring the builtin is installed), any other spec is resolved from
/// the referrer's source dir (or its recorded bundle source dir, for a generated bundle).
fn resolve_spec_lookup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    referrer: v8::Local<'s, v8::Module>,
    spec: &str,
) -> Option<PathBuf> {
    if is_test_api_module(spec) {
        ensure_vitest_builtin(scope);
        return Some(vitest_key());
    }
    if is_node_builtin(spec) {
        ensure_node_builtin(scope, spec);
        return Some(PathBuf::from(format!("<node:{spec}>")));
    }
    let ref_hash = referrer.get_identity_hash().get();
    REGISTRY.with(|r| {
        let reg = r.borrow();
        let from = reg.path_by_hash.get(&ref_hash)?;
        // A generated bundle lives in the cache dir (no node_modules beside it); resolve its
        // externalized bare imports relative to the recorded source dir instead.
        let base = reg
            .bundle_src_dir
            .get(from)
            .cloned()
            .or_else(|| from.parent().map(|p| p.to_path_buf()))?;
        resolve_spec(spec, &base)
    })
}

/// The already-registered ESM or CJS synthetic module for `abs`, if the registry has one.
fn loaded_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    let g = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.esm_by_path
            .get(abs)
            .or_else(|| reg.cjs_synth_by_path.get(abs))
            .cloned()
    })?;
    Some(v8::Local::new(scope, &g))
}

/// The REAL module an external package's static `import` of a vi.mock'ed ES module links
/// against: vitest leaves a package outside `server.deps.inline` to Node, which never sees a
/// mock (turbo-test#135 for require(), #218 for import). Loaded once with the mock records set
/// aside, kept in `real_esm` so the mock stays what app code (and an inlined package) gets.
/// `None` for app / inlined importers, unmocked targets, and CommonJS / asset / JSON targets.
fn real_esm_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    referrer: v8::Local<'s, v8::Module>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    let hash = referrer.get_identity_hash().get();
    let from = REGISTRY.with(|r| r.borrow().path_by_hash.get(&hash).cloned())?;
    let dir = from.parent()?;
    if !dir.to_string_lossy().contains("/node_modules/") || inlined_dep(dir) {
        return None;
    }
    if !REGISTRY.with(|r| r.borrow().mocks.contains_key(abs))
        || is_asset(abs)
        || abs.extension().and_then(|e| e.to_str()) == Some("json")
        || !matches!(detect_kind(abs), Kind::Esm)
    {
        return None;
    }
    if let Some(g) = REGISTRY.with(|r| r.borrow().real_esm.get(abs).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }
    let saved = REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        (
            reg.mocks.remove(abs),
            reg.cjs_exports.remove(abs),
            reg.cjs_synth_by_path.remove(abs),
            reg.cjs_export_names.remove(abs),
            reg.esm_by_path.remove(abs),
        )
    });
    let loaded = load_graph(scope, abs);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let real = reg.esm_by_path.remove(abs);
        let (m, e, s, n, esm) = saved;
        let p = abs.to_path_buf();
        if let Some(m) = m {
            reg.mocks.insert(p.clone(), m);
        }
        if let Some(e) = e {
            reg.cjs_exports.insert(p.clone(), e);
        }
        if let Some(s) = s {
            reg.cjs_synth_by_path.insert(p.clone(), s);
        }
        if let Some(n) = n {
            reg.cjs_export_names.insert(p.clone(), n);
        }
        if let Some(esm) = esm {
            reg.esm_by_path.insert(p.clone(), esm);
        }
        if let Some(real) = real.filter(|_| loaded.is_some()) {
            reg.real_esm.insert(p, real);
        }
    });
    REGISTRY.with(|r| {
        r.borrow()
            .real_esm
            .get(abs)
            .map(|g| v8::Local::new(scope, g))
    })
}

// ---- host callbacks: import.meta + dynamic import() -----------------------

extern "C" fn import_meta_callback(
    context: v8::Local<v8::Context>,
    module: v8::Local<v8::Module>,
    meta: v8::Local<v8::Object>,
) {
    v8::callback_scope!(unsafe scope, context);
    v8::scope!(let scope, scope);
    let hash = module.get_identity_hash().get();
    let path = REGISTRY.with(|r| r.borrow().path_by_hash.get(&hash).cloned());
    // vitest's members (filename / dirname / url / env / resolve / glob / main, the test file's
    // `vitest`), as the CJS paths' __ttImportMeta: runtime.js fills V8's object (turbo-test#123,
    // #130)
    let global = context.global(scope);
    let name = v8::String::new(scope, "__ttFillImportMeta").unwrap();
    let fill = global
        .get(scope, name.into())
        .and_then(|f| v8::Local::<v8::Function>::try_from(f).ok());
    // every module is registered by path when it is compiled (load_esm), before its code can
    // reach import.meta, and runtime.js defines __ttFillImportMeta
    if let (Some(p), Some(fill)) = (path, fill) {
        let file = v8::String::new(scope, &p.to_string_lossy()).unwrap();
        let undefined = v8::undefined(scope).into();
        fill.call(scope, undefined, &[meta.into(), file.into()]);
    }
}

fn dynamic_import_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    // vitest awaits a vi.doMock's async factory before an import() of its module resolves
    // (turbo-test#42 item 6): while one is pending (runtime.js __ttImportWait), the import runs
    // once it is registered
    let promise = match do_mock_wait(scope, resource_name, specifier) {
        Some(wait) => {
            let data = v8::Array::new_with_elements(scope, &[resource_name, specifier.into()]);
            let then = v8::Function::builder(import_after_do_mock)
                .data(data.into())
                .build(scope)?;
            wait.then(scope, then)?
        }
        None => dynamic_import(scope, resource_name, specifier)?,
    };
    track_import(scope, promise);
    Some(promise)
}

/// Hand an import()'s promise to runtime.js `__ttTrackImport` (vi.dynamicImportSettled waits
/// for the pending ones, turbo-test#79 item 2).
fn track_import(scope: &mut v8::PinScope, promise: v8::Local<v8::Promise>) {
    let global = scope.get_current_context().global(scope);
    let track = v8::String::new(scope, "__ttTrackImport")
        .and_then(|k| global.get(scope, k.into()))
        .and_then(|f| v8::Local::<v8::Function>::try_from(f).ok());
    if let Some(track) = track {
        let undefined = v8::undefined(scope).into();
        track.call(scope, undefined, &[promise.into()]);
    }
}

/// The pending async vi.doMock registration an import() of `specifier` from the module
/// `resource_name` waits for (runtime.js __ttImportWait), if any.
fn do_mock_wait<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__ttImportWait")?;
    let wait: v8::Local<v8::Function> = global.get(scope, key.into())?.try_into().ok()?;
    let referrer = resource_name.to_rust_string_lossy(scope);
    let dir = Path::new(&referrer)
        .parent()?
        .to_string_lossy()
        .into_owned();
    let dir = v8::String::new(scope, &dir)?;
    let undefined = v8::undefined(scope).into();
    let pending = wait.call(scope, undefined, &[dir.into(), specifier.into()])?;
    pending.try_into().ok()
}

/// The deferred import() of dynamic_import_callback: `[resource_name, specifier]` is its data.
fn import_after_do_mock(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).ok();
    let parts = data.and_then(|d| Some((d.get_index(scope, 0)?, d.get_index(scope, 1)?)));
    let parts = parts.and_then(|(r, s)| Some((r, v8::Local::<v8::String>::try_from(s).ok()?)));
    if let Some(promise) = parts.and_then(|(r, s)| dynamic_import(scope, r, s)) {
        rv.set(promise.into());
    }
}

/// The message and code of vitest's error for an import() of `spec` from `importer` that does
/// not resolve (`root`: the project root): a relative specifier is named as vite resolved it,
/// URL-resolved against the importer's root-relative path; an absolute one as written; a bare
/// one is a missing package; an unknown `node:` one is not a built-in.
fn import_not_found(
    spec: &str,
    importer: &str,
    root: &Path,
    as_written: bool,
) -> (String, &'static str) {
    if spec.starts_with("node:") {
        return (
            format!("No such built-in module: {spec}"),
            "ERR_UNKNOWN_BUILTIN_MODULE",
        );
    }
    let module = |id: &str| format!("Cannot find module '{id}' imported from {importer}");
    // a static import names the module as written
    if spec.starts_with('/') || (as_written && (spec.starts_with("./") || spec.starts_with("../")))
    {
        return (module(spec), "ERR_MODULE_NOT_FOUND");
    }
    if spec.starts_with("./") || spec.starts_with("../") {
        let importer = Path::new(importer);
        let dir = importer.strip_prefix(root).unwrap_or(importer).parent();
        let mut id: Vec<String> = dir
            .into_iter()
            .flat_map(|d| d.components())
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        for segment in spec.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    id.pop();
                }
                s => id.push(s.to_string()),
            }
        }
        return (
            module(&format!("/{}", id.join("/"))),
            "ERR_MODULE_NOT_FOUND",
        );
    }
    (
        format!("Cannot find package '{spec}' imported from {importer}"),
        "ERR_MODULE_NOT_FOUND",
    )
}

/// The namespace an import() of `abs` settles with: a guarded no-default mock, vitest's interop
/// namespace of a CommonJS module, an app ES module's names in source order, else `ns`.
fn import_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    is_builtin: bool,
    ns: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    // a CommonJS module (not mocked, a builtin, an asset or JSON): vitest's interop
    // namespace of it, as `import * as` reads it (turbo-test#133)
    let cjs =
        (!is_builtin && !is_asset(abs) && abs.extension().and_then(|e| e.to_str()) != Some("json"))
            .then(|| {
                REGISTRY.with(|r| {
                    let reg = r.borrow();
                    if reg.mocks.contains_key(abs) || reg.esm_by_path.contains_key(abs) {
                        return None;
                    }
                    reg.cjs_exports.get(abs).cloned()
                })
            })
            .flatten();
    let app_esm = !is_builtin
        && !abs.components().any(|c| c.as_os_str() == "node_modules")
        && REGISTRY.with(|r| {
            let reg = r.borrow();
            !reg.mocks.contains_key(abs) && reg.esm_by_path.contains_key(abs)
        });
    // a vi.mock factory's without `default`: the mock itself, whose `default` throws
    // vitest's error on every read (turbo-test#158)
    let guarded = REGISTRY
        .with(|r| {
            let reg = r.borrow();
            reg.mocks
                .contains_key(abs)
                .then(|| reg.cjs_exports.get(abs).cloned())
                .flatten()
        })
        .map(|g| v8::Local::new(scope, &g))
        .filter(|m| mock_guarded(scope, *m));
    Some(match cjs {
        _ if guarded.is_some() => guarded?,
        Some(g) => {
            let exports = v8::Local::new(scope, &g);
            variant_view(scope, exports, Variant::Ns)?
        }
        // an app ES module: its names in source order (turbo-test#141)
        None if app_esm => ordered_namespace(scope, abs, ns)?,
        None => ns,
    })
}

/// import()'s load of a non-builtin target (CJS under the module runner, else the ESM graph).
fn import_load_target(scope: &mut v8::PinScope, abs: &Path) -> Option<()> {
    // Module-runner: load app modules via the CJS transform path (live bindings, mocks,
    // re-runs after vi.resetModules). Falls back to the ESM graph loader otherwise.
    if mr_enabled() && esbuild_transform_cjs(abs, false).is_some() {
        load_cjs(scope, abs, true)?;
    } else {
        load_dep(scope, abs)?;
    }
    // a mock factory that threw: the import rejects with its error (vitest)
    if let Some(e) = mock_factory_error(scope, abs) {
        scope.throw_exception(e);
        return None;
    }
    Some(())
}

/// The module record import() of `abs` uses: a mocked module's synthetic wins over a real ES
/// module record of the same path (one imported before a vi.doMock): import() returns the mock.
fn import_module_record(abs: &Path) -> Option<v8::Global<v8::Module>> {
    REGISTRY.with(|r| {
        let reg = r.borrow();
        let mocked = reg.mocks.contains_key(abs);
        let synth = reg.cjs_synth_by_path.get(abs);
        if mocked && synth.is_some() {
            synth.cloned()
        } else {
            reg.esm_by_path.get(abs).or(synth).cloned()
        }
    })
}

/// import() of `specifier` from the module `resource_name`: a promise for its namespace.
fn dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);

    let referrer = resource_name.to_rust_string_lossy(scope);
    let from_dir = Path::new(&referrer).parent().map(|p| p.to_path_buf());
    let spec = specifier.to_rust_string_lossy(scope);

    // Layer B: configured barrel -> lazy Proxy namespace (await import('@mui/icons-material')).
    if let Some((key, def)) = lazy_stub_match(&spec) {
        if let Some(ns) = ensure_lazy_stub(scope, &key, def.as_deref()) {
            resolver.resolve(scope, ns);
            return Some(promise);
        }
    }
    let (abs, is_builtin) = dynamic_import_target(scope, &spec, from_dir.as_deref());
    let Some(abs) = abs else {
        reject_not_found(scope, &resolver, &spec, &referrer)?;
        return Some(promise);
    };

    // load + instantiate + evaluate the target, then resolve with its namespace. A load that
    // throws (the module body, a compile error) rejects with that error, caught here so it is
    // not left pending on the host callback.
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = &mut tc.init();
    let ns = (|| {
        if !is_builtin {
            import_load_target(scope, &abs)?;
        }
        // A mocked module's synthetic wins over a real ES module record of the same path (one
        // imported before a vi.doMock): import() returns the mock (vitest).
        let module = import_module_record(&abs)?;
        let module = v8::Local::new(scope, &module);
        if module.get_status() == v8::ModuleStatus::Uninstantiated {
            module.instantiate_module(scope, resolve_callback)?;
        }
        // evaluate() returns the module's evaluation promise (also for one already evaluated or
        // still evaluating its top-level await): the import settles with it (turbo-test#82)
        let evaluation = module.evaluate(scope)?;
        let ns = module.get_module_namespace();
        let ns = import_namespace(scope, &abs, is_builtin, ns)?;
        // (a synthetic module, e.g. a node builtin, evaluated before gives no promise)
        match v8::Local::<v8::Promise>::try_from(evaluation) {
            Ok(promise) => {
                let namespace = v8::Function::builder(return_data).data(ns).build(scope)?;
                promise.then(scope, namespace).map(Into::into)
            }
            Err(_) => Some(ns),
        }
    })();

    match ns {
        Some(ns) => {
            resolver.resolve(scope, ns);
        }
        None => {
            let exc = match scope.exception() {
                Some(e) => e,
                None => {
                    let err = v8::String::new(scope, &format!("dynamic import failed: {spec}"))?;
                    v8::Exception::error(scope, err)
                }
            };
            resolver.reject(scope, exc);
        }
    }
    Some(promise)
}

/// Resolve a `dynamic_import` specifier to a registry key plus whether it's a builtin (which
/// also ensures the builtin is installed). Dynamic imports of builtins such as turbo-dom's
/// `await import('node:module')` or vitest's API go through here.
fn dynamic_import_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    spec: &str,
    from_dir: Option<&Path>,
) -> (Option<PathBuf>, bool) {
    if is_test_api_module(spec) {
        ensure_vitest_builtin(scope);
        return (Some(vitest_key()), true);
    }
    if is_node_builtin(spec) {
        ensure_node_builtin(scope, spec);
        return (Some(PathBuf::from(format!("<node:{spec}>"))), true);
    }
    let abs = from_dir.and_then(|d| resolve_spec(spec, d));
    (abs.map(|a| query_mock_variant(spec, a)), false)
}

/// Reject `resolver` with the Node-style `ERR_MODULE_NOT_FOUND` for a missing dynamic import.
fn reject_not_found<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: &v8::Local<'s, v8::PromiseResolver>,
    spec: &str,
    referrer: &str,
) -> Option<()> {
    let root = std::env::current_dir().unwrap_or_default();
    let (message, code) = import_not_found(spec, referrer, &root, false);
    let message = v8::String::new(scope, &message)?;
    let exc = v8::Exception::error(scope, message);
    let key = v8::String::new(scope, "code")?;
    let code = v8::String::new(scope, code)?;
    exc.cast::<v8::Object>().set(scope, key.into(), code.into());
    resolver.reject(scope, exc);
    Some(())
}

/// Whether the registered mock `m` is a vi.mock factory's without `default` (runtime.js
/// `__ttMockGuarded`).
fn mock_guarded(scope: &mut v8::PinScope, m: v8::Local<v8::Value>) -> bool {
    let global = scope.get_current_context().global(scope);
    let f = v8::String::new(scope, "__ttMockGuarded")
        .and_then(|k| global.get(scope, k.into()))
        .and_then(|f| v8::Local::<v8::Function>::try_from(f).ok());
    let Some(f) = f else { return false };
    let undefined = v8::undefined(scope).into();
    f.call(scope, undefined, &[m])
        .is_some_and(|v| v.boolean_value(scope))
}

/// A function returning its data value (the namespace a dynamic import settles with).
fn return_data(
    _scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set(args.data());
}

// ---- bootstrap + public API ----------------------------------------------

fn bind_fn(
    scope: &mut v8::PinScope,
    target: v8::Local<v8::Object>,
    name: &str,
    f: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let tmpl = v8::FunctionTemplate::new(scope, f);
    let func = tmpl.get_function(scope).unwrap();
    let key = v8::String::new(scope, name).unwrap();
    target.set(scope, key.into(), func.into());
}

/// The test runtime + helpers (the "framework layer"). Baked into a V8 startup snapshot
/// once (M3 §4) so each test file gets a context *from* the snapshot instead of
/// re-evaluating this per file. (Minimal runtime now; real @vitest/* bundle swaps in here
/// behind the same snapshot mechanism.)
const RUNTIME_JS: &str = include_str!("runtime.js");

/// Build the framework snapshot once: a context with RUNTIME_JS evaluated, set as the
/// default context, serialized. Native callbacks (log/__nativeRequire) are NOT in the
/// snapshot — V8 can't serialize native function pointers — so they are re-bound per
/// context after deserialization (`install_natives`).
pub(crate) fn framework_snapshot() -> &'static FrameworkSnapshot {
    static SNAP: std::sync::OnceLock<FrameworkSnapshot> = std::sync::OnceLock::new();
    SNAP.get_or_init(|| {
        let chai = chai_script();
        // the blob is keyed on the flags it was built under
        let flags = run_v8_flags();
        if window_snapshot_enabled() {
            if let Some(snap) = window_snapshot(&chai, &flags) {
                turbo_dom_browser_env::browser_env::set_snapshot_install(true);
                return snap;
            }
        }
        let path = saved_blob_path("framework", blob_key_from("framework", 0, &chai, &flags));
        if let Some(data) = read_saved_blob(&path) {
            return FrameworkSnapshot {
                data,
                references: None,
            };
        }
        let mut creator = v8::Isolate::snapshot_creator(None, None);
        {
            v8::scope!(let scope, &mut creator);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            framework_prelude(scope);
            scope.set_default_context(context);
        }
        // E5: bake the framework's compiled bytecode into the snapshot (Keep) instead of dropping
        // it (Clear). Every per-file fresh isolate deserializes this snapshot; with Keep it gets
        // RUNTIME_JS already compiled (no per-isolate reparse/recompile of the framework layer).
        // Costs a larger blob (one-time, shared). env-gated for A/B. Microbench-valid (per-file
        // compile work, not concurrency).
        let handling = if std::env::var("TURBO_SNAP_KEEP").is_ok() {
            v8::FunctionCodeHandling::Keep
        } else {
            v8::FunctionCodeHandling::Clear
        };
        let data = creator.create_blob(handling).expect("framework snapshot");
        save_blob(&path, &data);
        FrameworkSnapshot {
            data: Box::leak(data.to_vec().into_boxed_slice()),
            references: None,
        }
    })
}

/// The window startup blob (turbo-dom#249): the one a previous process saved for this build,
/// reference table and inputs, else a new one, saved for the next process. A rebuilt blob is not
/// byte-identical, and V8 rejects bytecode caches made against another blob, so keeping the
/// blob is what lets every run reuse the on-disk module caches.
fn window_snapshot(chai: &str, flags: &str) -> Option<FrameworkSnapshot> {
    let (references, fingerprint) = turbo_dom_browser_env::browser_env::snapshot_references();
    let path = saved_blob_path("window", blob_key_from("window", fingerprint, chai, flags));
    if let Some(data) = read_saved_blob(&path) {
        return Some(FrameworkSnapshot {
            data,
            references: Some(references),
        });
    }
    let blob = turbo_dom_browser_env::browser_env::snapshot_blob_with(framework_prelude)?;
    // saved only when built with the recorded table (the key's fingerprint)
    let same = blob.references.len() == references.len()
        && blob
            .references
            .iter()
            .zip(references)
            .all(|(a, b)| unsafe { a.pointer == b.pointer });
    if same {
        save_blob(&path, blob.data);
    }
    Some(FrameworkSnapshot {
        data: blob.data,
        references: Some(blob.references),
    })
}

/// Where a startup blob of `kind` with `key` is kept: the build's cache dir (its name carries
/// the build id, so another binary never reads it).
fn saved_blob_path(kind: &str, key: u64) -> PathBuf {
    cache_dir().join(format!("snapshot-{kind}-{key:016x}.bin"))
}

/// The key of a saved startup blob: what it was built from beyond the build itself (its kind,
/// the reference table's fingerprint, chai's text and the V8 flags).
fn blob_key_from(kind: &str, fingerprint: u64, chai: &str, flags: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind, fingerprint, chai, flags).hash(&mut h);
    h.finish()
}

const SAVED_BLOB_MAGIC: &[u8; 8] = b"TTSNAP01";

/// Save a startup blob: magic, length, content hash, bytes (written atomically).
fn save_blob(path: &Path, data: &[u8]) {
    let mut out = Vec::with_capacity(data.len() + 24);
    out.extend_from_slice(SAVED_BLOB_MAGIC);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&blob_hash(data).to_le_bytes());
    out.extend_from_slice(data);
    write_atomic_bytes(path, &out);
}

/// A saved startup blob, leaked for the process, when it is intact (magic, length and hash
/// match): V8 aborts on a damaged blob, so anything else reads as absent.
fn read_saved_blob(path: &Path) -> Option<&'static [u8]> {
    let bytes = std::fs::read(path).ok()?;
    let (header, data) = bytes.split_at_checked(24)?;
    let len = u64::from_le_bytes(header[8..16].try_into().ok()?);
    let hash = u64::from_le_bytes(header[16..24].try_into().ok()?);
    let intact =
        &header[..8] == SAVED_BLOB_MAGIC && data.len() as u64 == len && blob_hash(data) == hash;
    intact.then(|| &*Box::leak(data.to_vec().into_boxed_slice()))
}

fn blob_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut h);
    h.finish()
}

/// The worker isolates' startup blob ([`framework_snapshot`]): the framework layer, and with
/// the window snapshot (`references` set) the window context at
/// `turbo_dom_browser_env::browser_env::SNAPSHOT_WINDOW_CONTEXT` too.
pub(crate) struct FrameworkSnapshot {
    data: &'static [u8],
    references: Option<&'static [v8::ExternalReference]>,
}

// SAFETY: the references are addresses of `extern "C"` functions (turbo-dom's table), immutable
// and valid process-wide; the data is an immutable leaked byte slice.
unsafe impl Send for FrameworkSnapshot {}
unsafe impl Sync for FrameworkSnapshot {}

impl FrameworkSnapshot {
    /// Isolate parameters that deserialize this blob.
    pub(crate) fn create_params(&self) -> v8::CreateParams {
        let params = v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(self.data));
        match self.references {
            Some(references) => params.external_references(std::borrow::Cow::Borrowed(references)),
            None => params,
        }
    }

    /// A new context of an isolate made from this blob: the window context for a DOM file
    /// when the blob has one, else the framework's default context.
    pub(crate) fn new_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        dom: bool,
    ) -> v8::Local<'s, v8::Context> {
        if dom && self.references.is_some() {
            let index = turbo_dom_browser_env::browser_env::SNAPSHOT_WINDOW_CONTEXT;
            // invariant: a blob with references is a snapshot_blob_with one, which holds it
            return v8::Context::from_snapshot(scope, index, Default::default())
                .expect("the window blob holds the window context");
        }
        v8::Context::new(scope, Default::default())
    }
}

/// Whether worker isolates come from the window snapshot (turbo-dom#249): `TURBO_SNAPSHOT=0`
/// turns it off (the framework-only blob; each DOM file then runs the engine install).
fn window_snapshot_enabled() -> bool {
    WINDOW_SNAPSHOT_WANTED.load(std::sync::atomic::Ordering::Relaxed)
        && window_snapshot_allowed(
            std::env::var("TURBO_SNAPSHOT").ok().as_deref(),
            std::env::var("TURBO_V8_FLAGS").ok().as_deref(),
        )
}

/// `TURBO_SNAPSHOT=0` keeps the framework-only blob, and so do custom V8 flags
/// (`TURBO_V8_FLAGS`, non-empty): code deserialized from the window blob can conflict with
/// them (`--interpreted-frames-native-stack` aborts V8).
fn window_snapshot_allowed(turbo_snapshot: Option<&str>, v8_flags: Option<&str>) -> bool {
    turbo_snapshot != Some("0") && v8_flags.is_none_or(str::is_empty)
}

/// Cleared by [`plan_window_snapshot`] for a run too small to repay the window snapshot.
static WINDOW_SNAPSHOT_WANTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// DOM files from which the window snapshot pays for itself: its one-time build (~170 ms on a
/// quiet M5) against the engine install it saves each DOM file (~20-30 ms).
const WINDOW_SNAPSHOT_MIN_DOM_FILES: usize = 8;

/// Whether a run with `dom_files` DOM-environment files builds the window snapshot.
fn window_snapshot_wanted(dom_files: usize) -> bool {
    dom_files >= WINDOW_SNAPSHOT_MIN_DOM_FILES
}

/// Before [`init_v8`]: build the window snapshot only when the run has enough DOM files to repay
/// it (a single-file or node-only run keeps the framework-only blob and its fast start).
pub fn plan_window_snapshot(files: &[PathBuf]) {
    let dom_files = files
        .iter()
        .filter(|f| needs_dom(f))
        .take(WINDOW_SNAPSHOT_MIN_DOM_FILES)
        .count();
    WINDOW_SNAPSHOT_WANTED.store(
        window_snapshot_wanted(dom_files),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The framework layer every worker context starts with: the host platform facts, chai and
/// RUNTIME_JS, evaluated in the snapshot context.
/// glibcVersionRuntime baked into the snapshot: empty (falsy) on musl, a glibc version otherwise.
#[cfg(target_env = "musl")]
const GLIBC_RUNTIME: &str = "";
#[cfg(not(target_env = "musl"))]
const GLIBC_RUNTIME: &str = "2.39";

fn framework_prelude(scope: &mut v8::PinScope) {
    // Bake the REAL host platform/arch (compile-time target) into the snapshot so
    // turbo-dom's index.js loader requires the matching prebuilt .node. Without this
    // the runtime reported 'darwin'/'arm64' everywhere and non-mac hosts loaded the
    // macOS .node -> dlopen fails -> "document is not defined".
    let node_platform = node_platform(std::env::consts::OS);
    let node_arch = node_arch(std::env::consts::ARCH);
    let os_type = node_os_type(std::env::consts::OS);
    // isMusl() reads glibcVersionRuntime: truthy => gnu, falsy => musl.
    let glibc_runtime = GLIBC_RUNTIME;
    let prelude = format!(
        "globalThis.__ttPlatform={node_platform:?};globalThis.__ttArch={node_arch:?};globalThis.__ttOsType={os_type:?};globalThis.__ttGlibc={glibc_runtime:?};"
    );
    let pcode = v8::String::new(scope, &prelude).unwrap();
    v8::Script::compile(scope, pcode, None)
        .unwrap()
        .run(scope)
        .unwrap();
    // chai (turbo-test#160): vitest's expect(x) is a chai Assertion; runtime.js loads it
    // through the globalThis.__ttLoadChai this defines
    let chai = chai_script();
    {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let ccode = v8::String::new(tc, &chai).unwrap();
        let ran = v8::Script::compile(tc, ccode, None).and_then(|s| s.run(tc));
        if ran.is_none() {
            let msg = caught_message(tc, "Error").unwrap_or_default();
            chai_missing(&format!("chai failed to load: {msg}"));
        }
    }
    let code = v8::String::new(scope, RUNTIME_JS).unwrap();
    let s = v8::Script::compile(scope, code, None).unwrap();
    s.run(scope).unwrap();
}

/// Where chai's ES-module build is: `TURBO_CHAI` (cli.js sets it to the `chai` this npm package
/// depends on); in this crate's unit tests, the repo's `node_modules/chai` (`npm ci`).
fn chai_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TURBO_CHAI") {
        return Some(PathBuf::from(p));
    }
    if cfg!(test) {
        return Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("node_modules/chai/index.js"));
    }
    None
}

/// No chai, no expect: say so and stop (never a silent fallback).
fn chai_missing(why: &str) -> ! {
    let msg = format!(
        "turbo-test: {why}\nturbo-test needs the `chai` package it depends on (vitest's expect is chai): \
         run it through its npm launcher (cli.js), which passes TURBO_CHAI, or set TURBO_CHAI to chai's index.js."
    );
    eprintln!("{msg}");
    std::process::exit(1);
}

/// chai's `index.js` (an ES module) lowered to CommonJS by the native ESM→CJS emitter, wrapped
/// into `globalThis.__ttLoadChai(EventTarget, Event)`, which runtime.js calls with private host
/// classes (chai's plugin events need them, apart from the DOM environment's: turbo-test#203) and
/// which returns chai's exports.
fn chai_script() -> String {
    let Some(path) = chai_path() else {
        chai_missing("TURBO_CHAI is not set");
    };
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| chai_missing(&format!("cannot read chai at {}: {e}", path.display())));
    let cjs = crate::esm_cjs::emit_with(&path, &src, false).unwrap_or_else(|| {
        chai_missing(&format!(
            "cannot lower chai at {} to CommonJS",
            path.display()
        ))
    });
    format!(
        "globalThis.__ttLoadChai = (EventTarget, Event) => {{ const module = {{ exports: {{}} }}; (function (module, exports) {{\n{cjs}\n}})(module, module.exports); return module.exports; }};"
    )
}

/// Node's `process.platform` for a Rust `std::env::consts::OS`.
fn node_platform(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        "windows" => "win32",
        other => other, // "linux", "freebsd", ...
    }
}

/// Node's `process.arch` for a Rust `std::env::consts::ARCH`.
fn node_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Node's `os.type()` for a Rust `std::env::consts::OS`.
fn node_os_type(os: &str) -> &str {
    match os {
        "macos" => "Darwin",
        "windows" => "Windows_NT",
        "linux" => "Linux",
        other => other,
    }
}

/// Where the test `console.*` forwarding (`[js] [log] …` lines, runtime.js `__emit` → native
/// `log`) goes: stderr when set. The CLI sets it when a machine artifact (json/junit/tap) owns
/// stdout, so test output can't corrupt the parsed artifact; human reporters keep it on stdout.
pub static CONSOLE_TO_STDERR: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// native `log(...parts)`: one `[js] <parts joined by spaces>` line (see [`CONSOLE_TO_STDERR`]).
fn native_log(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        parts.push(args.get(i).to_rust_string_lossy(scope));
    }
    if CONSOLE_TO_STDERR.load(Ordering::Relaxed) {
        eprintln!("[js] {}", parts.join(" "));
    } else {
        println!("[js] {}", parts.join(" "));
    }
}

/// native `__ttWrite(fd, text)`: `process.stdout.write` / `process.stderr.write` (runtime.js) —
/// the text as is, to stdout for fd 1 (stderr while [`CONSOLE_TO_STDERR`] is set, as console
/// output) and to stderr otherwise (turbo-test#70).
fn native_write(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    use std::io::Write;
    let fd = args.get(0).int32_value(scope).unwrap_or(1);
    let text = args.get(1).to_rust_string_lossy(scope);
    if fd == 1 && !CONSOLE_TO_STDERR.load(Ordering::Relaxed) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    } else {
        let _ = std::io::stderr().lock().write_all(text.as_bytes());
    }
}

/// native `__ttCallSite()`: the original `[line, column]` of the innermost stack frame in the test
/// file (location.rs), or `undefined` — where the runtime's `test(...)` was called from.
fn native_call_site(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let site = v8::StackTrace::current_stack_trace(scope, 64).and_then(|trace| {
        (0..trace.get_frame_count()).find_map(|i| {
            let frame = trace.get_frame(scope, i)?;
            let name = frame
                .get_script_name(scope)
                .map(|n| n.to_rust_string_lossy(scope))
                .unwrap_or_default();
            crate::location::resolve(&name, frame.get_line_number(), frame.get_column())
        })
    });
    if let Some((line, col)) = site {
        let items = [
            v8::Integer::new(scope, line as i32).into(),
            v8::Integer::new(scope, col as i32).into(),
        ];
        rv.set(v8::Array::new_with_elements(scope, &items).into());
    }
}

/// Re-bind native callbacks into a fresh (snapshot-derived) context.
/// os.tmpdir() as Node's: TMPDIR, else TMP, else TEMP, else /tmp, without a trailing slash.
fn node_tmpdir(var: impl Fn(&str) -> Option<String>) -> String {
    let dir = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|k| var(k).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "/tmp".to_string());
    match dir.strip_suffix('/') {
        Some(trimmed) if !trimmed.is_empty() => trimmed.to_string(),
        _ => dir,
    }
}

fn install_natives(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    bind_fn(scope, global, "log", native_log);
    bind_fn(scope, global, "__ttWrite", native_write);
    bind_fn(scope, global, "__ttCallSite", native_call_site);
    bind_fn(scope, global, "__nativeRequire", native_require);
    bind_fn(scope, global, "__ttRegisterMock", native_register_mock);
    bind_fn(scope, global, "__ttImportActual", native_import_actual);
    bind_fn(scope, global, "__ttResetModules", native_reset_modules);
    bind_fn(scope, global, "__ttUnmock", native_unmock);
    bind_fn(scope, global, "__ttResolveSpec", native_resolve_spec);
    bind_fn(scope, global, "__ttRequireResolve", native_require_resolve);
    bind_fn(scope, global, "__ttRequireStack", native_require_stack);
    bind_fn(scope, global, "__ttCache", native_require_cache);
    bind_fn(scope, global, "__fs_existsSync", fs_exists_sync);
    bind_fn(scope, global, "__fs_readdir", fs_readdir);
    bind_fn(scope, global, "__fs_stat", fs_stat);
    bind_fn(scope, global, "__fs_readFile", fs_read_file);
    bind_fn(scope, global, "__fs_writeFile", fs_write_file);
    bind_fn(scope, global, "__fs_mkdir", fs_mkdir);
    bind_fn(scope, global, "__fs_remove", fs_remove);
    bind_fn(scope, global, "__fs_symlinkSync", fs_symlink_sync);
    // process.cwd(): the process's working directory, as vitest's workers report it; os.tmpdir()
    // and os.homedir() as Node computes them
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned());
    for (name, value) in [
        ("__ttProcessCwd", cwd.ok()),
        ("__ttTmpdir", Some(node_tmpdir(|k| std::env::var(k).ok()))),
        ("__ttHomedir", std::env::var("HOME").ok()),
    ] {
        if let Some(value) = value {
            let key = v8::String::new(scope, name).unwrap();
            let value = v8::String::new(scope, &value).unwrap();
            global.set(scope, key.into(), value.into());
        }
    }
    bind_fn(
        scope,
        global,
        "__ttStructuredClone",
        crate::structured_clone::native_structured_clone,
    );
    bind_fn(
        scope,
        global,
        "__ttTextEncoding",
        crate::text_decoder::native_text_encoding,
    );
    bind_fn(
        scope,
        global,
        "__ttTextDecoderId",
        crate::text_decoder::native_text_decoder_id,
    );
    bind_fn(
        scope,
        global,
        "__ttTextDecode",
        crate::text_decoder::native_text_decode,
    );
    install_runtime_flags(scope);
    let prelude = runtime_prelude();
    if !prelude.is_empty() {
        if let Some(code) = v8::String::new(scope, &prelude) {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
    install_host_env(scope, global);
}

/// Stack / CI / snapshot-update / name-pattern globals for the runtime, from the CLI's env.
fn install_runtime_flags(scope: &mut v8::PinScope) {
    if std::env::var("TURBO_STACK").is_ok() {
        compile_and_run(scope, "globalThis.__TT_STACK=true");
    }
    // `-u/--update` → write/overwrite obsolete-or-missing snapshots instead of failing the
    // comparison (vitest's --update semantics). Plumbed via TURBO_UPDATE_SNAPSHOTS env (set by
    // the CLI) → globalThis.__TT_UPDATE_SNAPSHOTS, read by toMatchSnapshot in the runtime.
    // On CI (vitest: std-env's isCI) a missing snapshot is not written: it fails (turbo-test#73).
    if crate::ci::is_ci_env() {
        run_js(scope, "globalThis.__TT_CI=true;");
    }
    if std::env::var("TURBO_UPDATE_SNAPSHOTS").is_ok() {
        compile_and_run(scope, "globalThis.__TT_UPDATE_SNAPSHOTS=true");
    }
    // `-t/--testNamePattern` → expose the regex source to the runtime, which compiles it once and
    // skips non-matching tests. JSON-encode so any regex metacharacter survives the JS literal.
    if let Ok(pat) = std::env::var("TURBO_TEST_NAME_PATTERN") {
        let js = format!("globalThis.__TT_NAME_PATTERN={};", json_string(&pat));
        compile_and_run(scope, &js);
    }
}

/// Compile `src` and run it; silently drop a compile-time or runtime failure (the installer
/// paths are best-effort — a knob that fails to install leaves the runtime at its default).
fn compile_and_run(scope: &mut v8::PinScope, src: &str) {
    if let Some(code) = v8::String::new(scope, src) {
        if let Some(s) = v8::Script::compile(scope, code, None) {
            s.run(scope);
        }
    }
}

/// The runtime's execution-control prelude script (empty when no knob is set).
fn runtime_prelude() -> String {
    // Execution-control knobs from the CLI, plumbed to the runtime the same way as the name
    // pattern: env (set by cli.rs) → global the runtime reads. All optional.
    //   --testTimeout → __TT_DEFAULT_TIMEOUT (ms), --hookTimeout → __TT_HOOK_TIMEOUT (ms),
    //   --maxConcurrency → __TT_MAX_CONCURRENCY, --sequence.concurrent → __TT_SEQUENCE_CONCURRENT,
    //   --retry → __TT_DEFAULT_RETRY,
    //   --silent → __TT_SILENT, --no-allowOnly → __TT_FORBID_ONLY,
    //   --passWithNoTests → __TT_PASS_WITH_NO_TESTS.
    let mut prelude = String::new();
    // the numeric knobs (ignored unless a whole number). testTimeout / hookTimeout also accept
    // `Infinity` (turbo-test#273; vitest's withTimeout rule).
    for (var, global) in [
        ("TURBO_TEST_TIMEOUT", "__TT_DEFAULT_TIMEOUT"),
        ("TURBO_HOOK_TIMEOUT", "__TT_HOOK_TIMEOUT"),
        ("TURBO_MAX_CONCURRENCY", "__TT_MAX_CONCURRENCY"),
        ("TURBO_TEST_RETRY", "__TT_DEFAULT_RETRY"),
    ] {
        let Ok(v) = std::env::var(var) else { continue };
        if let Ok(n) = v.parse::<u64>() {
            prelude.push_str(&format!("globalThis.{global}={n};"));
        } else if v == "Infinity" && matches!(global, "__TT_DEFAULT_TIMEOUT" | "__TT_HOOK_TIMEOUT")
        {
            prelude.push_str(&format!("globalThis.{global}=Infinity;"));
        }
    }
    append_boolean_flags(&mut prelude);
    prelude
}

/// Emit `globalThis.__TT_X=true;` for every boolean knob the runtime reads from env/config.
fn append_boolean_flags(prelude: &mut String) {
    const ENV_FLAGS: &[(&str, &str)] = &[
        ("TURBO_SEQUENCE_CONCURRENT", "__TT_SEQUENCE_CONCURRENT"),
        ("TURBO_TEST_SILENT", "__TT_SILENT"),
        ("TURBO_TEST_FORBID_ONLY", "__TT_FORBID_ONLY"),
        // --reporter junit → each test's console output goes to the report (turbo-test#81)
        ("TURBO_CAPTURE_LOGS", "__TT_CAPTURE_LOGS"),
        // vitest's human reporters with colours on (turbo-test#94) → console values and diffs
        // are coloured as vitest's worker colours them
        ("TURBO_COLORS", "__TT_COLORS"),
        // --passWithNoTests (launcher) → a file with no tests at all passes instead of failing
        ("TURBO_PASS_WITH_NO_TESTS", "__TT_PASS_WITH_NO_TESTS"),
    ];
    for (var, global) in ENV_FLAGS {
        if std::env::var(var).is_ok() {
            prelude.push_str(&format!("globalThis.{global}=true;"));
        }
    }
    // --includeTaskLocation / test.includeTaskLocation → record each test's call site
    if crate::location::enabled() {
        prelude.push_str("globalThis.__TT_TASK_LOCATION=true;");
    }
}

/// process.env as vitest's (turbo-test#143, #156): the environment turbo-test was started with, in
/// its order (a non-UTF-8 name / value lossily decoded), then vitest's own variables. The host's
/// NODE_ENV wins (vitest only defaults it to 'test'); runtime.js's TEST / VITEST / BASE_URL / MODE
/// / VITEST_MODE / SSR / ... replace the host's; PROD is '1' (DEV '') under NODE_ENV=production;
/// FORCE_TTY is 'true' when stdout is a terminal. The object is filled in place: `node:process`'s
/// `env` and `import.meta.env` read this same object.
fn install_host_env(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    let host = v8::Object::new(scope);
    for (k, v) in std::env::vars_os() {
        // (a variable is far below V8's string limit: the environment is ARG_MAX-bounded)
        if let (Some(k), Some(v)) = (
            v8::String::new(scope, &k.to_string_lossy()),
            v8::String::new(scope, &v.to_string_lossy()),
        ) {
            host.set(scope, k.into(), v.into());
        }
    }
    let key = v8::String::new(scope, "__ttHostEnv").unwrap();
    global.set(scope, key.into(), host.into());
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        run_js(scope, "globalThis.process.env.FORCE_TTY = 'true';");
    }
    run_js(
        scope,
        "(() => { const e = globalThis.process.env, h = globalThis.__ttHostEnv; delete globalThis.__ttHostEnv; \
         const own = { ...e }; for (const k of Object.keys(e)) delete e[k]; Object.assign(e, h); \
         if (h.NODE_ENV !== undefined) delete own.NODE_ENV; Object.assign(e, own); \
         e.PROD = e.NODE_ENV === 'production' ? '1' : ''; e.DEV = e.PROD ? '' : '1'; })();",
    );
}

/// Minimal JSON string encoder for embedding a Rust string as a JS string literal.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// native fs.existsSync(path) -> bool
fn fs_exists_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let p = args.get(0).to_rust_string_lossy(scope);
    rv.set_bool(Path::new(&p).exists());
}

/// The Node error code and description of an OS error (`EEXIST`, "file already exists"), as
/// libuv names them; None for an error that is not one of these.
fn errno_name(e: &std::io::Error) -> Option<(&'static str, &'static str)> {
    const NAMES: [(i32, &str, &str); 6] = [
        (2, "ENOENT", "no such file or directory"),
        (13, "EACCES", "permission denied"),
        (17, "EEXIST", "file already exists"),
        (20, "ENOTDIR", "not a directory"),
        (1, "EPERM", "operation not permitted"),
        (21, "EISDIR", "illegal operation on a directory"),
    ];
    // ENOTEMPTY's number differs by OS (66 on macOS, 39 on Linux)
    if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
        return Some(("ENOTEMPTY", "directory not empty"));
    }
    let errno = e.raw_os_error()?;
    NAMES
        .iter()
        .find(|(n, _, _)| *n == errno)
        .map(|&(_, c, d)| (c, d))
}

/// native fs.symlinkSync(target, path): a symbolic link at `path` to `target`; a failure throws
/// Node's error (`EEXIST: file already exists, symlink 'target' -> 'path'`, its `code`).
fn fs_symlink_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let target = args.get(0).to_rust_string_lossy(scope);
    let path = args.get(1).to_rust_string_lossy(scope);
    #[cfg(unix)]
    let res = std::os::unix::fs::symlink(&target, &path);
    #[cfg(not(unix))]
    let res = std::os::windows::fs::symlink_file(&target, &path);
    if let Err(e) = res {
        let (code, desc) = errno_name(&e).unwrap_or(("EIO", "i/o error"));
        let msg = format!("{code}: {desc}, symlink '{target}' -> '{path}'");
        let msg = v8::String::new(scope, &msg).unwrap();
        let err = v8::Exception::error(scope, msg);
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(err) {
            for (k, v) in [
                ("code", code),
                ("syscall", "symlink"),
                ("path", &target),
                ("dest", &path),
            ] {
                let k = v8::String::new(scope, k).unwrap();
                let v = v8::String::new(scope, v).unwrap();
                obj.set(scope, k.into(), v.into());
            }
        }
        scope.throw_exception(err);
    }
}

/// Throw Node's error for a failed fs call: `ENOENT: no such file or directory, scandir '<path>'`
/// with its `errno` (negative), `code`, `syscall` and `path`.
fn throw_fs_error(scope: &mut v8::PinScope, e: &std::io::Error, syscall: &str, path: &str) {
    throw_fs_error_at(scope, e, syscall, Some(path));
}

/// throw_fs_error for a call Node reports without a path (`EISDIR: …, read`).
fn throw_fs_error_at(
    scope: &mut v8::PinScope,
    e: &std::io::Error,
    syscall: &str,
    path: Option<&str>,
) {
    let (code, desc) = errno_name(e).unwrap_or(("EIO", "i/o error"));
    let msg = match path {
        Some(path) => format!("{code}: {desc}, {syscall} '{path}'"),
        None => format!("{code}: {desc}, {syscall}"),
    };
    let msg = v8::String::new(scope, &msg).unwrap();
    let err = v8::Exception::error(scope, msg);
    let obj = err.cast::<v8::Object>();
    let errno = v8::Integer::new(scope, -e.raw_os_error().unwrap_or(5));
    let key = v8::String::new(scope, "errno").unwrap();
    obj.set(scope, key.into(), errno.into());
    let fields = [
        ("code", Some(code)),
        ("syscall", Some(syscall)),
        ("path", path),
    ];
    for (k, v) in fields.iter().filter_map(|(k, v)| Some((*k, (*v)?))) {
        let k = v8::String::new(scope, k).unwrap();
        let v = v8::String::new(scope, v).unwrap();
        obj.set(scope, k.into(), v.into());
    }
    scope.throw_exception(err);
}

/// native fs.readFile(path, text): the file's contents (a string, else a Uint8Array); Node's
/// `open` error, or `read` (no path) for a directory.
fn fs_read_file(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    use std::io::Read;
    let p = args.get(0).to_rust_string_lossy(scope);
    let mut file = match std::fs::File::open(&p) {
        Ok(f) => f,
        Err(e) => return throw_fs_error(scope, &e, "open", &p),
    };
    let mut bytes = Vec::new();
    if let Err(e) = file.read_to_end(&mut bytes) {
        return throw_fs_error_at(scope, &e, "read", None);
    }
    if args.get(1).boolean_value(scope) {
        let text = v8::String::new(scope, &String::from_utf8_lossy(&bytes)).unwrap();
        return rv.set(text.into());
    }
    let len = bytes.len();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    rv.set(v8::Uint8Array::new(scope, buffer, 0, len).unwrap().into());
}

/// native fs.writeFile(path, data, append): write (or append) a string or a byte view; Node's
/// `open` error (parents are not created).
fn fs_write_file(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    use std::io::Write;
    let p = args.get(0).to_rust_string_lossy(scope);
    let data = args.get(1);
    let bytes = match v8::Local::<v8::ArrayBufferView>::try_from(data) {
        Ok(view) => {
            let mut buf = vec![0; view.byte_length()];
            view.copy_contents(&mut buf);
            buf
        }
        Err(_) => data.to_rust_string_lossy(scope).into_bytes(),
    };
    let append = args.get(2).boolean_value(scope);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(&p);
    // (a failing write itself — a full disk — is reported as the open)
    if let Err(e) = file.and_then(|mut f| f.write_all(&bytes)) {
        throw_fs_error(scope, &e, "open", &p);
    }
}

/// native fs.mkdir(path, recursive): Node's `mkdir` error; recursive returns the first
/// directory it created, as given (`w/q` for `w/q/r/s`), or undefined.
fn fs_mkdir(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let p = args.get(0).to_rust_string_lossy(scope);
    if !args.get(1).boolean_value(scope) {
        if let Err(e) = std::fs::create_dir(&p) {
            throw_fs_error(scope, &e, "mkdir", &p);
        }
        return;
    }
    let first = Path::new(&p)
        .ancestors()
        .take_while(|a| !a.as_os_str().is_empty() && !a.exists())
        .last()
        .map(|a| a.to_string_lossy().into_owned());
    if let Err(e) = std::fs::create_dir_all(&p) {
        return throw_fs_error(scope, &e, "mkdir", &p);
    }
    if let Some(first) = first {
        rv.set(v8::String::new(scope, &first).unwrap().into());
    }
}

/// native fs.rmdir / unlink / rm-tree(path, op): remove a directory (`rmdir`), a file
/// (`unlink`) or a whole tree (`rm`), with Node's error for that syscall.
fn fs_remove(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let op = args.get(1).to_rust_string_lossy(scope);
    let res = match op.as_str() {
        "rmdir" => std::fs::remove_dir(&p),
        "unlink" => std::fs::remove_file(&p),
        _ => std::fs::remove_dir_all(&p),
    };
    if let Err(e) = res {
        throw_fs_error(scope, &e, &op, &p);
    }
}

/// The `S_IFMT` bits of a file type (Node's Dirent / Stats kinds; 0 when unknown).
fn file_type_bits(t: std::fs::FileType) -> u32 {
    #[cfg(unix)]
    use std::os::unix::fs::FileTypeExt;
    #[cfg(unix)]
    let special = [
        (t.is_block_device(), 0o060000),
        (t.is_char_device(), 0o020000),
        (t.is_fifo(), 0o010000),
        (t.is_socket(), 0o140000),
    ];
    #[cfg(not(unix))]
    let special = [];
    let kinds = [
        (t.is_symlink(), 0o120000),
        (t.is_dir(), 0o040000),
        (t.is_file(), 0o100000),
    ];
    special
        .iter()
        .chain(&kinds)
        .find(|k| k.0)
        .map_or(0, |k| k.1)
}

/// native fs.readdir(path) -> [name, S_IFMT bits][] in Node's (byte) order; a failure throws
/// Node's `scandir` error (runtime.js builds names or Dirents from it).
fn fs_readdir(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let entries = match std::fs::read_dir(&p) {
        Ok(entries) => entries,
        Err(e) => return throw_fs_error(scope, &e, "scandir", &p),
    };
    let mut list: Vec<(String, u32)> = entries
        .flatten()
        .map(|e| {
            let bits = e.file_type().map(file_type_bits).unwrap_or(0);
            (e.file_name().to_string_lossy().into_owned(), bits)
        })
        .collect();
    list.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let items: Vec<v8::Local<v8::Value>> = list
        .iter()
        .map(|(name, bits)| {
            let name = v8::String::new(scope, name).unwrap().into();
            let bits = v8::Integer::new_from_unsigned(scope, *bits).into();
            v8::Array::new_with_elements(scope, &[name, bits]).into()
        })
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &items).into());
}

/// native fs.stat(path, lstat) -> Node's Stats fields in order (dev, mode, nlink, uid, gid, rdev,
/// blksize, ino, size, blocks, atimeMs, mtimeMs, ctimeMs, birthtimeMs); a failure throws Node's
/// `stat` / `lstat` error.
fn fs_stat(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let lstat = args.get(1).boolean_value(scope);
    let res = if lstat {
        std::fs::symlink_metadata(&p)
    } else {
        std::fs::metadata(&p)
    };
    let m = match res {
        Ok(m) => m,
        Err(e) => return throw_fs_error(scope, &e, if lstat { "lstat" } else { "stat" }, &p),
    };
    let ms = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    };
    #[cfg(unix)]
    let fields = {
        use std::os::unix::fs::MetadataExt;
        let ctime = m.ctime() as f64 * 1000.0 + m.ctime_nsec() as f64 / 1e6;
        [
            m.dev() as f64,
            m.mode() as f64,
            m.nlink() as f64,
            m.uid() as f64,
            m.gid() as f64,
            m.rdev() as f64,
            m.blksize() as f64,
            m.ino() as f64,
            m.size() as f64,
            m.blocks() as f64,
            ms(m.accessed()),
            ms(m.modified()),
            ctime,
            ms(m.created()),
        ]
    };
    #[cfg(not(unix))]
    let fields = {
        let mode = if m.is_dir() { 0o040755 } else { 0o100644 };
        let modified = ms(m.modified());
        [
            0.0,
            mode as f64,
            1.0,
            0.0,
            0.0,
            0.0,
            4096.0,
            0.0,
            m.len() as f64,
            0.0,
            ms(m.accessed()),
            modified,
            modified,
            ms(m.created()),
        ]
    };
    let items: Vec<v8::Local<v8::Value>> = fields
        .iter()
        .map(|v| v8::Number::new(scope, *v).into())
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &items).into());
}

/// Node builtin module names we shim; each has an entry in runtime.js `__nodeBuiltins` (a unit
/// test checks that), served to `import` and `require` alike by `ensure_node_builtin`.
#[rustfmt::skip]
const NODE_BUILTINS: &[&str] = &[
    "fs", "fs/promises", "path", "module", "os", "child_process", "url", "util", "events",
    "stream", "assert", "assert/strict", "perf_hooks", "crypto", "buffer", "querystring",
    "string_decoder", "timers", "timers/promises", "http", "http2", "https", "net", "tls", "zlib",
    "dns", "dns/promises", "tty", "constants", "dgram", "readline", "v8", "diagnostics_channel",
    "process", "async_hooks", "worker_threads", "vm", "inspector",
];

fn is_node_builtin(spec: &str) -> bool {
    NODE_BUILTINS.contains(&spec.strip_prefix("node:").unwrap_or(spec))
}

/// Register a node builtin as a synthetic module sourced from globalThis.__nodeBuiltins[name].
fn ensure_node_builtin<'s>(scope: &mut v8::PinScope<'s, '_>, spec: &str) -> Option<()> {
    let key = PathBuf::from(format!("<node:{spec}>"));
    if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(&key)) {
        return Some(());
    }
    let global = scope.get_current_context().global(scope);
    let bk = v8::String::new(scope, "__nodeBuiltins")?;
    let builtins = v8::Local::<v8::Object>::try_from(global.get(scope, bk.into())?).ok()?;
    let lookup_name = spec.strip_prefix("node:").unwrap_or(spec);
    let nk = v8::String::new(scope, lookup_name)?;
    // every NODE_BUILTINS name has a shim; a missing one is a bug, not an empty module
    let exports_val = builtins
        .get(scope, nk.into())
        .filter(|v| !v.is_undefined())?;
    let names = object_keys(scope, global, exports_val);
    let exports_global = v8::Global::new(scope, exports_val);
    let synth = make_synthetic(scope, &names)?;
    let hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.cjs_exports.insert(key.clone(), exports_global);
        reg.cjs_export_names.insert(key.clone(), names);
        reg.cjs_synth_by_path.insert(key.clone(), synth_global);
        reg.path_by_hash.insert(hash, key);
    });
    Some(())
}

#[derive(Debug, Default)]
pub struct TestReport {
    pub passed: u32,
    pub failed: u32,
    pub failures: Vec<String>,
    /// Per-test list (full `describe > it` name, status, duration ms) — drives the
    /// per-testcase reporters (junit/tap/verbose). Empty only if the file failed to load.
    pub tests: Vec<TestCase>,
    /// One state per `describe` block: "pass" | "fail" | "skip" | "todo".
    pub suites: Vec<String>,
    /// The file's snapshot counts (`None`: it used no snapshot and has no snapshot file).
    pub snapshot: Option<SnapshotResult>,
    /// Per-file environment setup time (isolate+context-from-snapshot+natives), µs.
    pub setup_us: f64,
    /// The suites whose hooks failed, in vitest's suite order.
    pub suite_errors: Vec<SuiteError>,
    /// The describes without tasks (TAP tasks of their own, turbo-test#170): `index of the next
    /// test`, mode (`run` / `skip` / `todo`), then the describe path, joined by U+001F.
    pub empty_suites: Vec<String>,
    /// The file's console output while captured: (stream, task name path, content).
    pub logs: Vec<(String, String, String)>,
    /// ms the drive loop's clock jumped while the tests ran (turbo-test#95).
    pub waited_ms: f64,
}

/// A test file's snapshot outcome, as @vitest/snapshot's `SnapshotResult` (turbo-test#73).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapshotResult {
    pub added: u32,
    pub matched: u32,
    pub unmatched: u32,
    pub updated: u32,
    /// Obsolete snapshots (0 when the emptied file was deleted) and their keys.
    pub unchecked: u32,
    pub unchecked_keys: Vec<String>,
    /// The snapshot file was emptied (and, under -u, deleted).
    pub file_deleted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TestCase {
    /// The `describe > test` label (junit/tap/verbose).
    pub name: String,
    /// The test's own title and its enclosing `describe` names (vitest's `title` /
    /// `ancestorTitles`).
    pub title: String,
    pub ancestors: Vec<String>,
    /// "passed" | "failed" | "skipped" | "todo" | "pending" (a `.only` test inside describe.skip /
    /// describe.todo, which vitest never runs: turbo-test#77).
    pub status: String,
    pub duration_ms: f64,
    /// Real start time (epoch ms) of a test that ran; 0 otherwise.
    pub start_ms: f64,
    /// Failure message (empty unless status == "failed").
    pub message: String,
    /// The failure's error `stack` (else its `message`, else none: JSON `null`), as vitest's JSON
    /// `failureMessages` (turbo-test#97).
    pub failure_messages: Vec<Option<String>>,
    /// Where the test was registered (`includeTaskLocation`): 1-based line and column.
    pub location: Option<(u32, u32)>,
    /// A failed test that never ran (a forbidden `.only`, turbo-test#76): no `duration`.
    pub not_run: bool,
    /// A passing `it.fails` test: vitest's `expected fail` in the summary footer (turbo-test#78).
    pub expected_fail: bool,
    /// How many times it was retried (vitest's `retryCount`, `(retry x1)`).
    pub retry_count: u32,
    /// The last repeat it ran (vitest's `repeatCount`, `(repeat x2)` for `{ repeats: 2 }`).
    pub repeat_count: u32,
    /// Its console output (`stdout` / `stderr`, content) while logs are captured (junit).
    pub logs: Vec<(String, String)>,
    /// The error it failed with (junit's `<failure>`, turbo-test#81).
    pub errors: Vec<crate::failure::ErrorInfo>,
    /// The note of a test skipped at run time (`ctx.skip(note)`, turbo-test#96); empty for none.
    pub note: String,
    /// Its `ctx.annotate()` calls, in order (turbo-test#96).
    pub annotations: Vec<Annotation>,
}

/// A test annotation (`ctx.annotate(message, type)`): where it was made in the test file
/// (1-based line and column, 0 when unknown).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotation {
    pub kind: String,
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// A `describe` whose `beforeAll` / `afterAll` failed: a failed suite, reported by junit as a
/// test case of its own (turbo-test#81).
#[derive(Debug, Clone, Default)]
pub struct SuiteError {
    pub name: String,
    pub duration_ms: f64,
    pub errors: Vec<crate::failure::ErrorInfo>,
}

impl TestReport {
    /// Nothing failed — no failed test and no file-level failure entry. A file whose tests were
    /// all skipped / filtered out is ok (it counts as a passed file).
    pub fn ok(&self) -> bool {
        self.failed == 0 && self.failures.is_empty()
    }
}

/// Parse a `Profiler.takePreciseCoverage` result and attribute V8's per-range hit counts to
/// original source lines. Only project source files are reported — node_modules and test/spec
/// files are skipped (test files get hoist-reordered so their maps are unreliable, and they don't
/// belong in a coverage report anyway).
fn coverage_accumulate(json: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(scripts) = v
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|x| x.as_array())
    else {
        return;
    };
    for s in scripts {
        let Some(abs) = coverage_script_path(s) else {
            continue;
        };
        // Static map data (line table + source map) is built ONCE per file and reused across every
        // test file that covers it — rebuild the exact wrapper V8 compiled only on first encounter.
        if !crate::coverage::has_meta(abs) {
            if let Some(raw) = read_transformed(abs, true, false) {
                let wrapped = format!(
                    "(function (exports, module, require, __filename, __dirname) {{\n{raw}\n}})"
                );
                let orig = std::fs::read_to_string(abs).unwrap_or_default();
                crate::coverage::register_meta(abs, &wrapped, &orig);
            } else {
                continue;
            }
        }
        let (ranges, funcs) = coverage_script_ranges(s);
        if ranges.is_empty() {
            continue;
        }
        crate::coverage::map_script(abs, &ranges, &funcs);
        crate::coverage::map_branches(abs, &ranges);
        crate::coverage::map_statements(abs, &ranges);
    }
}

/// A precise-coverage script's file when it is an app source (absolute, existing, outside
/// node_modules, not a test / spec file).
fn coverage_script_path(s: &serde_json::Value) -> Option<&Path> {
    let url = s.get("url").and_then(|u| u.as_str())?;
    if url.is_empty() {
        return None;
    }
    let abs = Path::new(url);
    if !abs.is_absolute() || !abs.exists() {
        return None;
    }
    if abs.components().any(|c| c.as_os_str() == "node_modules") {
        return None;
    }
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.contains(".test.") || name.contains(".spec.") {
        return None;
    }
    Some(abs)
}

/// V8 precise-coverage byte ranges: `(start, end, count)`.
type CoverageRanges = Vec<(usize, usize, i64)>;
/// Covered functions: `(name, body start, call count)`.
type CoverageFuncs = Vec<(String, usize, i64)>;

/// The byte ranges `(start, end, count)` and named functions `(name, start, count)` of one
/// precise-coverage script.
fn coverage_script_ranges(s: &serde_json::Value) -> (CoverageRanges, CoverageFuncs) {
    let mut ranges: Vec<(usize, usize, i64)> = Vec::new();
    // (name, outer-range start offset, call count) — the FIRST range of each function is its
    // body span; its count is the invocation count → function coverage.
    let mut funcs: Vec<(String, usize, i64)> = Vec::new();
    {
        if let Some(fns) = s.get("functions").and_then(|f| f.as_array()) {
            for f in fns {
                let name = f
                    .get("functionName")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let rs = f.get("ranges").and_then(|r| r.as_array());
                if let Some(rs) = rs {
                    for (i, r) in rs.iter().enumerate() {
                        if let (Some(s0), Some(e0), Some(c0)) = (
                            r.get("startOffset").and_then(|x| x.as_u64()),
                            r.get("endOffset").and_then(|x| x.as_u64()),
                            r.get("count").and_then(|x| x.as_i64()),
                        ) {
                            if i == 0 {
                                // a top-level wrapper-ish function with no name + huge span is the
                                // module body itself — skip from function metrics, keep its ranges.
                                if !name.is_empty() {
                                    funcs.push((name.clone(), s0 as usize, c0));
                                }
                            }
                            ranges.push((s0 as usize, e0 as usize, c0));
                        }
                    }
                }
            }
        }
    }
    (ranges, funcs)
}

/// The caught exception's text (`fallback` when V8 has none to give), or None if nothing was
/// caught.
fn caught_message(
    tc: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope<'_>>>,
    fallback: &str,
) -> Option<String> {
    if !tc.has_caught() {
        return None;
    }
    Some(
        tc.exception()
            .map(|e| e.to_rust_string_lossy(tc))
            .unwrap_or_else(|| fallback.to_string()),
    )
}

/// Load-error text for an entry module whose `instantiate_module` did not succeed.
fn instantiate_error(caught: Option<String>) -> String {
    format!(
        "instantiate: {}",
        caught.unwrap_or_else(|| "instantiate failed".to_string())
    )
}

/// Load-error text (if any) for an entry module after `evaluate` + a microtask checkpoint:
/// a caught exception, an errored module (its exception, fetched lazily), or any other
/// not-evaluated outcome.
fn evaluate_error(
    caught: Option<String>,
    evaluated: bool,
    status: v8::ModuleStatus,
    exception: impl FnOnce() -> String,
) -> Option<String> {
    if let Some(e) = caught {
        return Some(format!("evaluate threw: {e}"));
    }
    if evaluated && status == v8::ModuleStatus::Evaluated {
        return None;
    }
    if status == v8::ModuleStatus::Errored {
        Some(format!("module errored: {}", exception()))
    } else {
        Some(format!("evaluate failed: status={status:?}"))
    }
}

/// Run a single test file end-to-end and return its pass/fail report.
/// Whether a mock's target may be externalized from an esbuild bundle (setup mocks; entry mocks
/// under TURBO_NO_MR) so the mock registry intercepts it. Not for (a) partial mocks (spread the
/// real module via importOriginal/importActual) and (b) factories that build React elements —
/// those must use the TEST's react instance, but a prepass/setup bundle has its own react
/// (dual-react render fail). Both stay un-externalized so the real module bundles with the
/// test's react. JSX is an arrow body starting with `<` (optionally parenthesized); an arrow
/// returning a parenthesized object literal (`() => ({ ... })`, the common factory form) is
/// not React.
fn mock_externalizable(factory: &str) -> bool {
    let is_partial = factory.contains("importOriginal") || factory.contains("importActual");
    let jsx_arrow = factory.match_indices("=>").any(|(at, _)| {
        let body = factory[at + 2..].trim_start();
        body.strip_prefix('(')
            .unwrap_or(body)
            .trim_start()
            .starts_with('<')
    });
    let builds_react = factory.contains("react/jsx")
        || factory.contains("createElement")
        || factory.contains("React.")
        || jsx_arrow;
    !is_partial && !builds_react
}

pub fn run_test_file(entry: &Path) -> Result<TestReport, String> {
    LOAD_ERROR.with(|e| e.borrow_mut().take());
    // Per-file native-DOM lifecycle: reset the thread-local DOM (fresh document per file) and, when
    // debugging (TURBO_RUST_DOM_LOG), dump the missing-member access log — on ANY exit incl. a load
    // failure (the surface React/testing-library touched is recorded during the failed load).
    // A file on the reuse path keeps the worker's document: it is installed once per worker, as
    // vitest `isolate: false` keeps one environment (turbo-test#150), and dropped with the worker.
    struct DomGuard;
    impl Drop for DomGuard {
        fn drop(&mut self) {
            if on_reuse_path() && DOM_INSTALLED.with(|d| d.get()) {
                return;
            }
            if turbo_dom_browser_env::browser_env::enabled() {
                turbo_dom_browser_env::browser_env::reset();
            }
        }
    }
    let _dom_guard = DomGuard;

    let entry_abs = crate::canon::canonicalize(entry).map_err(|e| format!("{e}"))?;
    crate::location::begin(
        &entry_abs,
        &std::fs::read_to_string(&entry_abs).unwrap_or_default(),
    );
    TEST_FILE.with(|f| *f.borrow_mut() = entry_abs.clone());
    // e2e helper tests may import heavy Node-only deps they don't exercise — allow stubbing a
    // failed node_modules load for these files only (see stub_failed_deps).
    ENTRY_LENIENT.with(|c| c.set(entry_abs.components().any(|p| p.as_os_str() == "e2e")));
    // CommonJS-first resolution for jest/node backends (sequelize/tslib/lexical get their working
    // require-condition build). Decided once per file from the project; vitest/React → ESM-first.
    CJS_FIRST_RESOLUTION.with(|c| c.set(cjs_first_project(&entry_abs)));

    // Setup-file mock specifiers are kept OUT of the esbuild bundle so the module boundary
    // survives and the (reliably-drained) mock registry intercepts them. Entry-file mocks are
    // NOT externalized: their factories can fail to register (JSX/importOriginal), and a
    // dangling external would fail the whole file — safer to leave them bundled.
    let setup_files = vitest_setup_files(&entry_abs);
    SCOPED_SETUP_FILES.with(|f| *f.borrow_mut() = setup_files.clone());
    let (mut externals, mut mock_targets) = setup_mock_externals(&setup_files);

    // TURBO_NO_MR (legacy bundle path): the ENTRY file's own mock targets are externalized the
    // same way (turbo-test#31 item 15). Bundled into the entry, a mocked module was unreachable
    // by the mock registry, so entry vi.mock/jest.mock never intercepted anything. Externalized
    // and rewritten to its absolute path, the import resolves through the registry, where
    // run_entry_mocks' prepass registered the mock before the bundle links. The same
    // exclusions as for setup mocks apply (see mock_externalizable): React-building and
    // partial (importOriginal/importActual) factories stay bundled, a documented limitation of
    // this escape hatch; the default module-runner mocks every module per-module.
    // (Module-runner mode loads no bundle, so nothing is externalized there.)
    if !mr_enabled() {
        add_entry_mock_externals(&entry_abs, &mut externals, &mut mock_targets);
    }

    // Bundle deps with esbuild (Vite-style) so the whole dep tree loads as one clean ESM;
    // fall back to the native per-module loader if esbuild is unavailable or fails.
    // Module-runner mode skips the bundle entirely: the entry + its require-graph load
    // per-module as CJS (live bindings, shared react, in-realm mocks).
    let load_target = if mr_enabled() {
        entry_abs.clone()
    } else {
        esbuild_bundle_full(&entry_abs, &externals, &mock_targets, None)
            .unwrap_or_else(|| entry_abs.clone())
    };

    // Make mock synthetics lenient: collect the names the bundle imports from each mock so
    // the synthetic exposes them (missing-from-factory ones export undefined, like vitest).
    if !mr_enabled() {
        register_lenient_mock_names(&load_target, &mock_targets);
    }

    let plan = EntryPlan {
        setup_files: &setup_files,
        externals: &externals,
        mock_targets: &mock_targets,
        load_target: &load_target,
    };
    // Zero-config speed path: reuse this worker's isolate+context across files (vitest
    // isolate:false semantics) so node_modules barrels evaluate once, not per file. Enabled by
    // the project's vitest config (`isolate: false`) or TURBO_REUSE_ISOLATE; off with TURBO_NO_REUSE.
    // FORCE_FRESH (the fresh-retry) overrides reuse for this one file. Coverage always runs the
    // fresh path (one isolate per file) — that's where the inspector collector is wired, and it
    // keeps the byte-offset → source mapping unambiguous.
    if !FORCE_FRESH.with(|f| f.get()) && reuse_decision(&entry_abs) && !crate::coverage::enabled() {
        return run_test_file_reused(&entry_abs, &plan);
    }

    run_test_file_isolated(&entry_abs, &plan)
}

/// Setup-file mock specifiers to externalize from the bundle, and each one's mock target by
/// basename.
fn setup_mock_externals(
    setup_files: &[PathBuf],
) -> (Vec<String>, std::collections::HashMap<String, PathBuf>) {
    let mut externals: Vec<String> = Vec::new();
    let mut mock_targets: std::collections::HashMap<String, PathBuf> =
        std::collections::HashMap::new();
    for sf in setup_files {
        let sf_dir = sf.parent().unwrap_or(Path::new("."));
        let sf_src = std::fs::read_to_string(sf).unwrap_or_default();
        for (s, factory) in extract_mocks(&sf_src) {
            // a glob (e.g. '*?url') is not a module; a `?query` id is one of its own (mock_key
            // keeps the query, so the rewritten import reaches that mock, not the plain
            // module's: turbo-test#42 item 5), externalized only when its file exists
            if s.contains('*') || !mock_externalizable(&factory) {
                continue;
            }
            match mock_key(&s, sf_dir) {
                Some(abs) => {
                    mock_targets.entry(spec_basename(&s)).or_insert(abs);
                }
                None if s.contains('?') => continue,
                None => {}
            }
            if !externals.contains(&s) {
                externals.push(s);
            }
        }
    }
    (externals, mock_targets)
}

/// TURBO_NO_MR: externalize the entry file's own (externalizable) mock targets too.
fn add_entry_mock_externals(
    entry_abs: &Path,
    externals: &mut Vec<String>,
    mock_targets: &mut std::collections::HashMap<String, PathBuf>,
) {
    let entry_dir = entry_abs.parent().unwrap_or(Path::new("."));
    let entry_src = std::fs::read_to_string(entry_abs).unwrap_or_default();
    // vi.doMock targets too: a later `import()` of one must reach the registry, not a copy
    // esbuild bundled into the entry (turbo-test#39 item 1)
    let calls = extract_mocks(&entry_src)
        .into_iter()
        .chain(extract_do_mocks(&entry_src));
    for (s, factory) in calls {
        // a glob is not a module; a `?query` id is one of its own (mock_key keeps the query,
        // so the rewritten import reaches that mock, not the plain module's)
        if s.contains('*') || !mock_externalizable(&factory) {
            continue;
        }
        let Some(abs) = mock_key(&s, entry_dir) else {
            continue;
        };
        mock_targets.entry(spec_basename(&s)).or_insert(abs);
        if !externals.contains(&s) {
            externals.push(s);
        }
    }
}

/// Make mock synthetics lenient: collect the names the bundle imports from each mock so the
/// synthetic exposes them (missing-from-factory ones export undefined, like vitest).
fn register_lenient_mock_names(
    load_target: &Path,
    mock_targets: &std::collections::HashMap<String, PathBuf>,
) {
    if let Ok(btext) = std::fs::read_to_string(load_target) {
        for abs in mock_targets.values() {
            let names = collect_named_imports(&btext, &abs.to_string_lossy());
            if !names.is_empty() {
                REGISTRY.with(|r| {
                    r.borrow_mut()
                        .extra_exports
                        .entry(abs.clone())
                        .or_default()
                        .extend(names)
                });
            }
        }
    }
}

/// __cwd: the test's project root (nearest package.json outside node_modules), the runner's
/// internal base (process.cwd() is the process's working directory, #54).
fn set_project_cwd(scope: &mut v8::PinScope, entry_abs: &Path) {
    let mut root = entry_abs.parent();
    let mut found: Option<&Path> = None;
    while let Some(d) = root {
        if d.join("package.json").is_file() && !d.to_string_lossy().contains("node_modules") {
            found = Some(d);
            break;
        }
        root = d.parent();
    }
    if let Some(r) = found {
        let js = format!("globalThis.__cwd = {:?};", r.to_string_lossy());
        if let Some(code) = v8::String::new(scope, &js) {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
}

/// Apply the config's runtime settings for this file (fakeTimers, zeroDelayTimers, mock / stub
/// switches); a literal that does not evaluate is left out.
fn apply_runtime_config(scope: &mut v8::PinScope, entry_abs: &Path) {
    if let Some(cfg) = vitest_runtime_config(entry_abs) {
        run_js(
            scope,
            &format!("try {{ globalThis.__ttApplyConfig(({cfg})); }} catch (e) {{}}"),
        );
    }
}

/// Reset the vi.hoisted index (keep the cache the prepass filled) so the entry's
/// `const x = vi.hoisted(...)` calls reuse the SAME objects the mock factories closed over, by
/// re-hitting the cache in source order.
fn reset_hoisted_idx(scope: &mut v8::PinScope) {
    let g = scope.get_current_context().global(scope);
    if let Some(k) = v8::String::new(scope, "__hoistedIdx") {
        let zero = v8::Integer::new(scope, 0);
        g.set(scope, k.into(), zero.into());
    }
}

/// Load + run the test module (its mocks already registered), then drive its tests.
fn load_and_drive_entry(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    entry_abs: &Path,
    load_target: &Path,
) -> Result<TestReport, String> {
    // Module-runner: load the entry as CJS (esbuild --format=cjs transform); its body
    // (describe/it + inline vi.mock) runs in-realm via the CJS wrapper, with the
    // require-graph loaded per-module (live bindings, shared react). No ESM bundle.
    // Only when the entry can actually be CJS-transformed (esbuild project present);
    // standalone ESM fixtures (no esbuild) fall through to the legacy ESM graph loader.
    if mr_enabled() && esbuild_transform_cjs(entry_abs, false).is_some() {
        return load_and_drive_cjs_entry(scope, global, entry_abs);
    }
    let module = load_entry_graph(scope, load_target)?;
    {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let ok = module.instantiate_module(tc, resolve_callback);
        if ok != Some(true) {
            let exc = tc.exception();
            note_load_error(tc, exc);
            return Err(instantiate_error(caught_message(tc, "unknown")));
        }
    }
    let ev_err: Option<String> = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let ev = module.evaluate(tc);
        tc.perform_microtask_checkpoint();
        let caught = caught_message(tc, "unknown");
        let exc = match module.get_status() {
            v8::ModuleStatus::Errored => Some(module.get_exception()),
            _ => tc.exception(),
        };
        note_load_error(tc, exc);
        evaluate_error(caught, ev.is_some(), module.get_status(), || {
            module.get_exception().to_rust_string_lossy(tc)
        })
    };
    if let Some(e) = ev_err {
        return Err(e);
    }
    // 3. drive the collected tests: __tt.run() -> Promise<summary>
    drive_tests(scope, global, entry_abs)
}

/// The module-runner entry: load it as CJS, then drive its tests.
fn load_and_drive_cjs_entry(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    entry_abs: &Path,
) -> Result<TestReport, String> {
    let err = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        if load_cjs(tc, entry_abs, false).is_none() {
            let exc = tc.exception();
            note_load_error(tc, exc);
            Some(
                caught_message(tc, "entry load failed")
                    .unwrap_or_else(|| "entry load failed".into()),
            )
        } else {
            tc.perform_microtask_checkpoint();
            None
        }
    };
    if let Some(e) = err {
        return Err(e);
    }
    // Persist __ttDir = the entry file's dir for the test-run phase (loads cleared it).
    // So vi.doMock/vi.unmock/vi.importActual fired from inside a test callback resolve
    // relative + `@/`-aliased specs against the test file, like at hoist time.
    if let Some(d) = entry_abs.parent() {
        let js = format!("globalThis.__ttDir = {:?};", d.to_string_lossy());
        if let Some(code) = v8::String::new(scope, &js) {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
    drive_tests(scope, global, entry_abs)
}

/// Fresh path: run one file in its own isolate booted from the framework snapshot.
fn run_test_file_isolated(entry_abs: &Path, plan: &EntryPlan) -> Result<TestReport, String> {
    // All Global handles live in the thread-local REGISTRY; they MUST be dropped while
    // this isolate is still alive. So every exit path runs through clear_registry()
    // before the isolate is torn down.
    // Per-file isolation via snapshot: a fresh isolate booted from the framework
    // snapshot; Context::new yields the baked default context (framework already
    // present). We time setup (isolate+context+natives) — the M3/M0 guardrail.
    let setup_start = std::time::Instant::now();
    let isolate = &mut v8::Isolate::new(isolate_params());
    isolate.set_host_initialize_import_meta_object_callback(import_meta_callback);
    isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
    crate::heap_guard::install(isolate);
    crate::heap_guard::arm();
    // Coverage: attach an inspector to this isolate BEFORE entering scopes (needs &mut isolate).
    let mut collector =
        crate::coverage::enabled().then(|| crate::coverage::Collector::new(isolate));

    v8::scope!(let scope, isolate);
    let dom = needs_dom(entry_abs);
    let context = framework_snapshot().new_context(scope, dom);
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    install_natives(scope, global);
    // Begin precise coverage now — captures module-load + test-execution counts.
    if let Some(c) = collector.as_mut() {
        c.start(context);
    }
    set_project_cwd(scope, entry_abs);
    let setup_us = setup_start.elapsed().as_secs_f64() * 1_000_000.0;

    discard_stale_code_caches();
    let _pending_guard = PendingCodeCacheGuard;
    set_file_env(scope, entry_abs);
    let result = run_fresh_file(scope, global, dom, entry_abs, plan);
    drop_pending_code_caches();

    // A file that ran into the isolate's heap limit was terminated by the heap guard: report
    // that, not whatever the half-unwound run happened to say.
    let result = if crate::heap_guard::tripped() {
        Err(crate::heap_guard::oom_error(
            scope.get_heap_statistics().heap_size_limit(),
        ))
    } else {
        result
    };
    if crate::heap_guard::log_enabled() {
        log_heap(scope, entry_abs, false);
    }

    // Coverage: read out V8's precise counts for everything this file executed, then map the
    // byte ranges back to original source lines. Done before teardown (isolate still alive).
    if let Some(c) = collector.as_mut() {
        if let Some(json) = c.take() {
            coverage_accumulate(&json);
        }
        c.stop(context);
    }
    drop(collector); // drop the inspector while the isolate + scope are still alive

    // drop all Globals while the isolate is still alive
    clear_registry();
    result.map(|mut r| {
        r.setup_us = setup_us;
        r
    })
}

/// The fresh path's per-file work: DOM, config, setup files, serializers, entry mocks, then
/// the entry itself.
fn run_fresh_file(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    dom: bool,
    entry_abs: &Path,
    plan: &EntryPlan,
) -> Result<TestReport, String> {
    // 0. DOM environment (turbo-dom) — only for files that need it, so pure-logic
    //    tests keep their clean globals (mirrors vitest per-file environment).
    if dom {
        setup_dom(scope, entry_abs);
    }
    // 0a. the config's test.fakeTimers (what vi.useFakeTimers() uses, turbo-test#180) and
    //     mock / stub switches (turbo-test#183), for the setup files too (a literal that
    //     does not evaluate is left out, as vitest would fail earlier)
    apply_runtime_config(scope, entry_abs);
    // 0b. project setupFiles: register jest-dom matchers (expect.extend) AND any
    //     global vi.mock()s, draining each into the path-keyed mock registry.
    for sf in plan.setup_files {
        run_setup_file(scope, sf, plan.externals, plan.mock_targets)?;
    }
    // 0c. the config's snapshotSerializers (turbo-test#167), after the setup files
    let serializers = vitest_snapshot_serializers(entry_abs);
    if !serializers.is_empty() {
        load_snapshot_serializers(scope, &serializers)?;
    }
    // 1. entry-file vi.mock(): hoisted + registered before instantiation, via a
    //    transformed pre-pass (handles JSX/factory transforms), drained to the
    //    path-keyed mock registry. Its specifiers were externalized from the bundle.
    run_entry_mocks(scope, entry_abs)?;
    reset_hoisted_idx(scope);
    // 2. load + run the test module.
    load_and_drive_entry(scope, global, entry_abs, plan.load_target)
}

/// What a test file runs with besides its entry: setup files, the bundle's externalized mock
/// specifiers and their targets, and the module to load (the entry or its bundle).
struct EntryPlan<'a> {
    setup_files: &'a [PathBuf],
    externals: &'a [String],
    mock_targets: &'a std::collections::HashMap<String, PathBuf>,
    load_target: &'a Path,
}

/// Reuse path: install the natives on a worker's first file, else reset the previous file's
/// app modules, mocks and per-file framework state.
fn prepare_reused_context(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    first_file: bool,
) {
    if first_file {
        install_natives(scope, global);
    } else {
        // drop app modules + test-file mocks (keep node_modules evals + setup mocks), then
        // wipe per-file framework state (tests/spies/timers/mocks/hoist/stubs/DOM document).
        let stale = unevaluated_modules(scope);
        reset_app_registry(&stale);
        // undo any prior file's re-mock of a setup path (analytics etc.)
        restore_setup_mocks(scope);
        if let Some(code) = v8::String::new(
            scope,
            "globalThis.__ttResetForNextFile && globalThis.__ttResetForNextFile();",
        ) {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
}

/// Reuse path: run the worker's setup files once (first file) and snapshot the hook / config
/// baseline and the setup-registered mocks.
fn run_setup_files_once(scope: &mut v8::PinScope, plan: &EntryPlan) {
    let setup_err = plan
        .setup_files
        .iter()
        .find_map(|sf| run_setup_file(scope, sf, plan.externals, plan.mock_targets).err());
    REUSE_SETUP_ERROR.with(|e| *e.borrow_mut() = setup_err);
    SETUP_MOCKS.with(|s| {
        *s.borrow_mut() = REGISTRY.with(|r| r.borrow().mocks.keys().cloned().collect());
    });
    if let Some(code) = v8::String::new(
        scope,
        "globalThis.__ttCaptureHookBaseline && globalThis.__ttCaptureHookBaseline(); \
         globalThis.__ttCaptureConfigBaseline && globalThis.__ttCaptureConfigBaseline();",
    ) {
        if let Some(s) = v8::Script::compile(scope, code, None) {
            s.run(scope);
        }
    }
    snapshot_setup_mocks(scope);
}

/// The reuse path's per-file work: DOM (once per worker), setup files (once), entry mocks, then
/// the entry itself.
fn run_reused_file(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    entry_abs: &Path,
    first_file: bool,
    plan: &EntryPlan,
) -> Result<TestReport, String> {
    // DOM: install once per worker and keep it across its files, as vitest `isolate: false`
    // keeps one environment (turbo-test#150); a fresh-isolate run or a recycle reinstalls.
    if needs_dom(entry_abs) && !DOM_INSTALLED.with(|d| d.get()) {
        setup_dom(scope, entry_abs);
        DOM_INSTALLED.with(|d| d.set(true));
    }
    // Setup files run ONCE per worker (vitest isolate:false semantics): their hooks,
    // matchers and mocks persist across files. After the first run we snapshot the hook
    // baseline (restored each file) and the setup-registered mock paths (kept on reset).
    if first_file {
        run_setup_files_once(scope, plan);
    }
    // a setup file that threw (on this worker's first file) fails every file after it too
    if let Some(e) = REUSE_SETUP_ERROR.with(|e| e.borrow().clone()) {
        return Err(e);
    }
    run_entry_mocks(scope, entry_abs)?;
    reset_hoisted_idx(scope);
    load_and_drive_entry(scope, global, entry_abs, plan.load_target)
}

/// Heap guard after a reused file: (tripped the limit, the limit, replace the isolate).
/// A file that hit the limit was terminated mid-run: report the OOM and throw the isolate away.
/// Otherwise a reused isolate only ever gets bigger (every file's module graph and DOM stays
/// reachable by design, like vitest's `isolate: false`), so once its live heap passes the
/// threshold after a full GC it is replaced and the next file boots a new one (re-running the
/// setup files), instead of growing until V8 aborts the process.
fn reuse_heap_check(scope: &mut v8::PinScope, entry_abs: &Path) -> (bool, usize, bool) {
    let limit = scope.get_heap_statistics().heap_size_limit();
    let tripped = crate::heap_guard::tripped();
    let threshold = crate::heap_guard::recycle_threshold(limit);
    let mut recycle = tripped;
    if !tripped
        && (crate::heap_guard::log_enabled()
            || scope.get_heap_statistics().used_heap_size() > threshold / 2)
    {
        scope.low_memory_notification();
        recycle = scope.get_heap_statistics().used_heap_size() > threshold;
        if crate::heap_guard::log_enabled() {
            log_heap(scope, entry_abs, recycle);
        }
    }
    (tripped, limit, recycle)
}

/// Create params for a worker isolate: the framework snapshot, plus an explicit heap limit when
/// `TURBO_MAX_HEAP_MB` is set.
fn isolate_params() -> v8::CreateParams {
    let params = framework_snapshot().create_params();
    match crate::heap_guard::max_heap_bytes() {
        Some(max) => params.heap_limits(0, max),
        None => params,
    }
}

/// `TURBO_HEAP_LOG`: the isolate's heap after a full GC, one stderr line per file.
fn log_heap(scope: &mut v8::PinScope, entry: &Path, recycled: bool) {
    scope.low_memory_notification();
    let st = scope.get_heap_statistics();
    let modules = REGISTRY.with(|r| {
        let r = r.borrow();
        r.cjs_exports.len() + r.esm_by_path.len()
    });
    eprintln!(
        "[heap] {:?} used={}MB total={}MB limit={}MB modules={} recycled={} {}",
        std::thread::current().id(),
        crate::heap_guard::mb(st.used_heap_size()),
        crate::heap_guard::mb(st.total_heap_size()),
        crate::heap_guard::mb(st.heap_size_limit()),
        modules,
        recycled,
        entry.display()
    );
}

/// Reuse path: run one file on this worker's persistent isolate+context. node_modules modules
/// stay evaluated across files; per-file framework + app state is reset between files.
fn run_test_file_reused(entry_abs: &Path, plan: &EntryPlan) -> Result<TestReport, String> {
    let setup_start = std::time::Instant::now();
    // init the persistent isolate once per worker
    REUSE_ISO.with(|iso_cell| {
        if iso_cell.borrow().is_none() {
            let mut isolate = v8::Isolate::new(isolate_params());
            isolate.set_host_initialize_import_meta_object_callback(import_meta_callback);
            isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
            crate::heap_guard::install(&mut isolate);
            *iso_cell.borrow_mut() = Some(isolate);
        }
    });

    crate::heap_guard::arm();
    // (result, replace this worker's isolate before the next file)
    let (outcome, recycle) = REUSE_ISO.with(|iso_cell| {
        let mut iso_b = iso_cell.borrow_mut();
        let isolate = iso_b.as_mut().unwrap();
        v8::scope!(let scope, isolate);

        // get-or-create the persistent context (framework snapshot already baked in)
        let (context, first_file) = REUSE_CTX.with(|c| {
            let mut cb = c.borrow_mut();
            if let Some(g) = cb.as_ref() {
                (v8::Local::new(scope, g), false)
            } else {
                let ctx = framework_snapshot().new_context(scope, needs_dom(entry_abs));
                *cb = Some(v8::Global::new(scope, ctx));
                (ctx, true)
            }
        });
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        prepare_reused_context(scope, global, first_file);
        // the config's runtime settings for this file (zeroDelayTimers, fakeTimers, mock / stub
        // switches), as the fresh path applies them; the reset above restored the defaults
        apply_runtime_config(scope, entry_abs);
        // __cwd: the test's project root (nearest package.json outside node_modules).
        set_project_cwd(scope, entry_abs);
        let setup_us = setup_start.elapsed().as_secs_f64() * 1_000_000.0;

        discard_stale_code_caches();
        let _pending_guard = PendingCodeCacheGuard;
        set_file_env(scope, entry_abs);
        let result = run_reused_file(scope, global, entry_abs, first_file, plan);
        drop_pending_code_caches();

        // Heap guard. A file that hit the limit was terminated mid-run: report the OOM and throw
        // the isolate away. Otherwise a reused isolate only ever gets bigger (every file's module
        // graph and DOM stays reachable by design, like vitest's `isolate: false`), so once its
        // live heap passes the threshold after a full GC it is replaced and the next file boots a
        // new one (re-running the setup files), instead of growing until V8 aborts the process.
        let (tripped, limit, recycle) = reuse_heap_check(scope, entry_abs);
        let result = if tripped {
            Err(crate::heap_guard::oom_error(limit))
        } else {
            result
        };
        // NOTE: no clear_registry() here — node_modules Globals stay alive for the next file;
        // app + mock entries are pruned at the top of the next file (reset_app_registry).
        (
            result.map(|mut r| {
                r.setup_us = setup_us;
                r
            }),
            recycle,
        )
    });
    if recycle {
        end_worker_reuse();
    }
    outcome
}

/// End-of-worker cleanup for the reuse path: drop registry Globals + the context while the
/// isolate is still alive, then drop the isolate. Avoids a shutdown assert from handles
/// outliving the isolate. No-op when reuse was never used.
pub fn end_worker_reuse() {
    if REUSE_ISO.with(|i| i.borrow().is_none()) {
        return;
    }
    clear_registry();
    // Globals: drop them while the isolate is still alive (a recycled isolate is followed by more
    // files on this thread, so a handle outliving its isolate would not stay harmless).
    SETUP_MOCK_SNAPSHOT.with(|s| s.borrow_mut().clear());
    // The worker's document (kept across its files, turbo-test#150) goes with its isolate.
    if DOM_INSTALLED.with(|d| d.get()) && turbo_dom_browser_env::browser_env::enabled() {
        turbo_dom_browser_env::browser_env::reset();
    }
    REUSE_CTX.with(|c| {
        let _ = c.borrow_mut().take();
    });
    REUSE_ISO.with(|i| {
        let _ = i.borrow_mut().take();
    });
    DOM_INSTALLED.with(|d| d.set(false));
    SETUP_MOCKS.with(|s| s.borrow_mut().clear());
    REUSE_SETUP_ERROR.with(|e| e.borrow_mut().take());
}

/// Whether isolate-reuse was selected for this run (env or vitest config). False until the first
/// non-forced file resolves the decision.
pub fn is_reuse_enabled() -> bool {
    reuse_isolate_enabled()
}

/// Authoritative fresh-isolate retry: run `entry` on a FRESH per-file isolate even under reuse.
/// A file whose run under reuse is not final (`cli::reuse_result_is_final`, turbo-test#126: a
/// test failing with something other than an assertion, a load error, a panic, an out-of-memory
/// abort) may have hit the reused isolate's own state; re-running it in a clean isolate is the
/// source of truth. The worker's reuse REGISTRY
/// is swapped out for the duration so the fresh isolate's Globals don't clobber the persistent
/// reuse module cache (the saved registry's Globals stay valid — the persistent isolate lives on).
pub fn run_test_file_fresh(entry: &Path) -> Result<TestReport, String> {
    run_fresh_with(entry, run_test_file)
}

/// `run_test_file_fresh` with the per-file runner injected (unit tests drive the panic path).
fn run_fresh_with(
    entry: &Path,
    run: impl FnOnce(&Path) -> Result<TestReport, String>,
) -> Result<TestReport, String> {
    let saved = REGISTRY.with(|r| std::mem::take(&mut *r.borrow_mut()));
    FORCE_FRESH.with(|f| f.set(true));
    // Panic-safe: a V8/loader panic must NOT lose the saved reuse registry or strand FORCE_FRESH.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(entry)));
    FORCE_FRESH.with(|f| f.set(false));
    // The fresh run installed (and dropped) its own document in this thread's DOM slot, which
    // the worker's document shares: the next reused file installs the worker's document again.
    DOM_INSTALLED.with(|d| d.set(false));
    // On success the fresh path already cleared its registry; on panic it may hold handles to the
    // now-dead fresh isolate. Either way, swap the saved reuse registry back in and FORGET the
    // fresh one (don't drop — its isolate is gone, dropping would assert).
    REGISTRY.with(|r| {
        let fresh_reg = std::mem::replace(&mut *r.borrow_mut(), saved);
        std::mem::forget(fresh_reg);
    });
    match result {
        Ok(r) => r,
        Err(_) => Err("panicked (fresh retry)".to_string()),
    }
}

fn drive_tests(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    entry_abs: &Path,
) -> Result<TestReport, String> {
    let report = drive_tests_bounded(scope, global, entry_abs, 2_000_000);
    // a run the heap guard stopped (its isolate is terminated) or that errored writes no cache
    if report.is_err() || crate::heap_guard::tripped() {
        drop_pending_code_caches();
    } else {
        flush_code_caches(scope);
    }
    report
}

/// vitest's `task.file.name` (turbo-test#96): the test file relative to the run's root (both
/// canonical), as the reports name it; its canonical path for a file outside the root.
pub fn task_file_name(entry: &Path, root: &Path) -> String {
    let canon = |p: &Path| crate::canon::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let entry = canon(entry);
    match entry.strip_prefix(canon(root)) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => entry.to_string_lossy().into_owned(),
    }
}

/// Expose the test file's absolute path to the runtime so toMatchSnapshot() can locate the
/// `__snapshots__/<basename>.snap` sibling (mirrors vitest's per-file snapshot resolution).
fn set_test_file_globals(scope: &mut v8::PinScope, entry_abs: &Path) {
    {
        let root = std::env::var_os("TURBO_ROOT")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let js = format!(
            "globalThis.__ttFile = {:?}; globalThis.__ttFileName = {}; globalThis.__ttMetaOfTestFile?.();",
            entry_abs.to_string_lossy(),
            json_string(&task_file_name(entry_abs, &root))
        );
        if let Some(code) = v8::String::new(scope, &js) {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
}

/// Drive the event loop with Node ordering until run()'s promise settles: drain
/// process.nextTick + Promise microtasks to a fixpoint, then run ONE macrotask (timer), and
/// repeat. Between every macrotask the micro/nextTick queues fully drain. Err unless fulfilled.
fn run_event_loop(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    promise: v8::Local<v8::Promise>,
    max_turns: u32,
) -> Result<(), String> {
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > max_turns {
            return Err("event loop did not settle (possible infinite timers)".into());
        }
        drain_microtasks(scope, global);
        // The task and its microtasks are done: settle the DOM ownership work a long run of
        // mutating calls on a large tree deferred, so what the task detached is collectable
        // before the next one (turbo-dom#112).
        if turbo_dom_browser_env::browser_env::enabled() {
            turbo_dom_browser_env::browser_env::flush_ownership(scope);
        }
        if promise.state() != v8::PromiseState::Pending {
            break;
        }
        if step_macrotask(scope, global) {
            continue;
        }
        break;
    }
    finalize_promise_state(scope, promise)
}

/// Drain `process.nextTick` plus Promise microtasks to a fixpoint. Each pass runs a batch of
/// nextTicks (through the runtime hook), then one microtask checkpoint; the loop ends when a
/// pass neither ran a nextTick nor left one pending.
fn drain_microtasks(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    loop {
        let ran = call_global_bool(scope, global, "__drainNextTicks");
        scope.perform_microtask_checkpoint();
        let pending = call_global_bool(scope, global, "__hasNextTicks");
        if !ran && !pending {
            return;
        }
    }
}

/// Run one macrotask: a pending native-addon callback for this thread first (while addon work is
/// in flight it keeps the loop alive, as libuv's handles do, waiting up to the next timer's
/// delay), then the runtime's `__stepMacro`. Returns `true` when something ran and the outer
/// loop should spin again, `false` when nothing was pending.
fn step_macrotask(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) -> bool {
    if crate::napi_host::run_native_task(scope) {
        return true;
    }
    if crate::napi_host::keeps_loop_alive(scope) {
        let delay = call_global_number(scope, global, "__ttNextMacroDelay");
        let timeout = (delay >= 0.0).then(|| std::time::Duration::from_secs_f64(delay / 1000.0));
        if crate::napi_host::wait_native_task(scope, timeout) {
            return true;
        }
    }
    call_global_bool(scope, global, "__stepMacro")
}

/// Convert the final promise state into the `run_event_loop` result: `Ok(())` on fulfilled, a
/// `rejected: …` / `still pending` message otherwise.
fn finalize_promise_state(
    scope: &mut v8::PinScope,
    promise: v8::Local<v8::Promise>,
) -> Result<(), String> {
    if promise.state() == v8::PromiseState::Fulfilled {
        return Ok(());
    }
    let reason = if promise.state() == v8::PromiseState::Rejected {
        format!(
            "rejected: {}",
            promise.result(scope).to_rust_string_lossy(scope)
        )
    } else {
        "still pending".to_string()
    };
    Err(format!("test run promise did not fulfill ({reason})"))
}

/// Per-test records (runtime.js `__testRecord`) of the run summary, for the per-testcase
/// reporters.
fn test_cases_of(scope: &mut v8::PinScope, summary: v8::Local<v8::Object>) -> Vec<TestCase> {
    let mut tests = Vec::new();
    let tkey = v8::String::new(scope, "tests").unwrap();
    if let Some(arr) = summary
        .get(scope, tkey.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
    {
        for i in 0..arr.length() {
            if let Some(o) = arr
                .get_index(scope, i)
                .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
            {
                tests.push(test_case_of(scope, o));
            }
        }
    }
    tests
}

/// One runtime.js test record as a `TestCase`.
fn test_case_of<'s>(scope: &mut v8::PinScope<'s, '_>, o: v8::Local<'s, v8::Object>) -> TestCase {
    let text = |scope: &mut v8::PinScope, key: &str| -> String {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into())
            .filter(|v| !v.is_undefined() && !v.is_null())
            .map(|v| v.to_rust_string_lossy(scope))
            .unwrap_or_default()
    };
    let num = |scope: &mut v8::PinScope, key: &str| -> f64 {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into())
            .filter(|v| !v.is_undefined())
            .and_then(|v| v.number_value(scope))
            .unwrap_or(0.0)
    };
    let flag = |scope: &mut v8::PinScope, key: &str| -> bool {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into()).is_some_and(|v| v.is_true())
    };
    TestCase {
        name: text(scope, "name"),
        title: text(scope, "title"),
        ancestors: string_array(scope, o, "ancestors"),
        status: text(scope, "status"),
        duration_ms: num(scope, "duration_ms"),
        start_ms: num(scope, "start_ms"),
        message: text(scope, "message"),
        failure_messages: js_array(scope, o, "failure_messages")
            .into_iter()
            .map(|v| (!v.is_null()).then(|| v.to_rust_string_lossy(scope)))
            .collect(),
        location: location_of(scope, o),
        not_run: flag(scope, "not_run"),
        expected_fail: flag(scope, "expected_fail"),
        retry_count: num(scope, "retry_count") as u32,
        repeat_count: num(scope, "repeat_count") as u32,
        logs: pairs_of(scope, o, "logs"),
        errors: errors_of(scope, o),
        note: text(scope, "note"),
        annotations: string_tuples(scope, o, "annotations")
            .into_iter()
            .map(|a| {
                let at = |i: usize| a.get(i).cloned().unwrap_or_default();
                Annotation {
                    kind: at(0),
                    message: at(1),
                    line: at(2).parse().unwrap_or(0),
                    col: at(3).parse().unwrap_or(0),
                }
            })
            .collect(),
    }
}

/// `drive_tests` with the event-loop turn budget injected (unit tests use a tiny one).
fn drive_tests_bounded(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    entry_abs: &Path,
    max_turns: u32,
) -> Result<TestReport, String> {
    set_test_file_globals(scope, entry_abs);
    let tt_key = v8::String::new(scope, "__tt").unwrap();
    let tt = v8::Local::<v8::Object>::try_from(global.get(scope, tt_key.into()).ok_or("no __tt")?)
        .map_err(|_| "__tt not object")?;
    let run_key = v8::String::new(scope, "run").unwrap();
    let run = v8::Local::<v8::Function>::try_from(tt.get(scope, run_key.into()).ok_or("no run")?)
        .map_err(|_| "run not fn")?;
    let result = run.call(scope, tt.into(), &[]).ok_or("run() threw")?;

    // run() returns a Promise<summary>. Drive the event loop with Node ordering:
    // drain process.nextTick + Promise microtasks to a fixpoint, then run ONE macrotask
    // (timer), and repeat. Between every macrotask the micro/nextTick queues fully drain.
    let promise = v8::Local::<v8::Promise>::try_from(result).map_err(|_| "run() not promise")?;
    run_event_loop(scope, global, promise, max_turns)?;
    let summary = v8::Local::<v8::Object>::try_from(promise.result(scope))
        .map_err(|_| "summary not object")?;

    let get_num = |scope: &mut v8::PinScope, key: &str| -> u32 {
        let k = v8::String::new(scope, key).unwrap();
        summary
            .get(scope, k.into())
            .and_then(|v| v.number_value(scope))
            .unwrap_or(0.0) as u32
    };
    let passed = get_num(scope, "passed");
    let failed = get_num(scope, "failed");

    let failures = string_array(scope, summary, "failures");
    // One state per describe block ('pass' / 'fail' / 'skip' / 'todo').
    let suites = string_array(scope, summary, "suites");
    // Per-test records (runtime.js `__testRecord`) for the per-testcase reporters.
    let tests = test_cases_of(scope, summary);
    let snapshot = snapshot_of(scope, summary);
    let suite_errors = suite_errors_of(scope, summary);
    let empty_suites = string_array(scope, summary, "empty_suites");
    let logs = string_tuples(scope, summary, "logs")
        .into_iter()
        .map(|mut l| {
            l.resize(3, String::new());
            (l.remove(0), l.remove(0), l.remove(0))
        })
        .collect();
    Ok(TestReport {
        passed,
        failed,
        failures,
        tests,
        suites,
        snapshot,
        setup_us: 0.0,
        suite_errors,
        empty_suites,
        logs,
        waited_ms: f64::from(get_num(scope, "waited_ms")),
    })
}

/// The items of the array `obj[key]` (none when it is not an array).
fn js_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
) -> Vec<v8::Local<'s, v8::Value>> {
    let k = v8::String::new(scope, key).unwrap();
    let arr = obj
        .get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok());
    arr.map(|a| {
        (0..a.length())
            .filter_map(|i| a.get_index(scope, i))
            .collect()
    })
    .unwrap_or_default()
}

/// The string items of the arrays in `obj[key]` (a test's `logs` pairs).
fn string_tuples<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for v in js_array(scope, obj, key) {
        if let Ok(item) = v8::Local::<v8::Array>::try_from(v) {
            let fields = (0..item.length())
                .map(|i| {
                    item.get_index(scope, i)
                        .map(|v| v.to_rust_string_lossy(scope))
                        .unwrap_or_default()
                })
                .collect();
            out.push(fields);
        }
    }
    out
}

/// `[a, b]` string pairs of the array `obj[key]` (a test's `logs`).
fn pairs_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
) -> Vec<(String, String)> {
    string_tuples(scope, obj, key)
        .into_iter()
        .map(|mut p| {
            p.resize(2, String::new());
            (p.remove(0), p.remove(0))
        })
        .collect()
}

/// The `errors` of a test record or a suite error (runtime.js `__errorInfo` objects).
fn errors_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> Vec<crate::failure::ErrorInfo> {
    let mut out = Vec::new();
    for v in js_array(scope, obj, "errors") {
        if let Ok(o) = v8::Local::<v8::Object>::try_from(v) {
            out.push(error_info_of(scope, o));
        }
    }
    out
}

/// An `__errorInfo` object as an [`ErrorInfo`](crate::failure::ErrorInfo): its stack's frames
/// in the test file are mapped back to the original source (location.rs).
fn error_info_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    o: v8::Local<'s, v8::Object>,
) -> crate::failure::ErrorInfo {
    let text = |key: &str| {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into())
            .filter(|v| !v.is_undefined() && !v.is_null())
            .map(|v| v.to_rust_string_lossy(scope))
            .unwrap_or_default()
    };
    let (name, message, diff, stack) = (text("name"), text("message"), text("diff"), text("stack"));
    let flag = |scope: &mut v8::PinScope<'s, '_>, key: &str| {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into()).is_some_and(|v| v.is_true())
    };
    let (no_name, no_message) = (flag(scope, "no_name"), flag(scope, "no_message"));
    let optional = |scope: &mut v8::PinScope<'s, '_>, key: &str| {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into())
            .filter(|v| v.is_string())
            .map(|v| v.to_rust_string_lossy(scope))
    };
    let (inspected, properties) = (optional(scope, "inspected"), optional(scope, "properties"));
    let (actual, expected) = (optional(scope, "actual"), optional(scope, "expected"));
    let show_diff = flag(scope, "show_diff");
    let frames = crate::failure::parse_stack(&stack)
        .into_iter()
        .map(|mut f| {
            let script = f.file.to_string_lossy().into_owned();
            if let Some((line, col)) =
                crate::location::resolve_frame(&script, f.line as usize, f.col as usize)
            {
                (f.line, f.col) = (line, col);
            }
            f
        })
        .collect();
    let k = v8::String::new(scope, "cause").unwrap();
    let cause = o
        .get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .map(|c| Box::new(error_info_of(scope, c)));
    let k = v8::String::new(scope, "rollup").unwrap();
    let rollup = o
        .get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .map(|r| {
            let text = |key: &str| {
                let k = v8::String::new(scope, key).unwrap();
                r.get(scope, k.into())
                    .map(|v| v.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            };
            crate::failure::RollupDetails {
                plugin: text("plugin"),
                id: text("id"),
                loc: text("loc"),
                frame: text("frame"),
            }
        });
    crate::failure::ErrorInfo {
        name,
        message,
        no_name,
        no_message,
        inspected,
        properties,
        diff,
        frames,
        cause,
        source: None,
        rollup,
        show_diff,
        actual,
        expected,
    }
}

/// The run summary's `suite_errors` (runtime.js `runSuite`; unfilled slots are suites that did
/// not fail).
fn suite_errors_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    summary: v8::Local<'s, v8::Object>,
) -> Vec<SuiteError> {
    let mut out = Vec::new();
    for v in js_array(scope, summary, "suite_errors") {
        if let Ok(o) = v8::Local::<v8::Object>::try_from(v) {
            let k = v8::String::new(scope, "name").unwrap();
            let name = o
                .get(scope, k.into())
                .map(|v| v.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let k = v8::String::new(scope, "duration_ms").unwrap();
            let duration_ms = o
                .get(scope, k.into())
                .and_then(|v| v.number_value(scope))
                .unwrap_or(0.0);
            out.push(SuiteError {
                name,
                duration_ms,
                errors: errors_of(scope, o),
            });
        }
    }
    out
}

thread_local! {
    /// The error the running file failed to load with (its entry, instantiation or evaluation),
    /// for junit's synthetic test case (turbo-test#81).
    static LOAD_ERROR: RefCell<Option<crate::failure::ErrorInfo>> = const { RefCell::new(None) };
}

/// Record the exception `exc` a file's load failed with (runtime.js `__ttErrorInfo` gives its
/// name, message, diff and stack).
fn note_load_error(scope: &mut v8::PinScope, exc: Option<v8::Local<v8::Value>>) {
    let Some(exc) = exc else {
        return;
    };
    let global = scope.get_current_context().global(scope);
    let info = v8::String::new(scope, "__ttErrorInfo")
        .and_then(|k| global.get(scope, k.into()))
        .and_then(|f| v8::Local::<v8::Function>::try_from(f).ok())
        .and_then(|f| {
            let undefined = v8::undefined(scope).into();
            f.call(scope, undefined, &[exc])
        })
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok());
    if let Some(info) = info {
        let mut info = error_info_of(scope, info);
        // A module that failed to compile has no frame to show: vitest fails the import before
        // the importer runs (its stack is the loader's own), where the module runner's require()
        // of the module leaves the importer's frame (turbo-test#99).
        let compile = COMPILE_EXC.with(|c| c.borrow_mut().take());
        if let Some((c, module)) = compile {
            if v8::Local::new(scope, &c).strict_equals(exc) {
                info.frames.clear();
                info.source = Some(module);
            }
        }
        LOAD_ERROR.with(|e| *e.borrow_mut() = Some(info));
    }
}

/// The error the last file run on this thread failed to load with, if any.
pub fn take_load_error() -> Option<crate::failure::ErrorInfo> {
    LOAD_ERROR.with(|e| e.borrow_mut().take())
}

/// The run summary's `snapshot` counts (runtime.js `__snapFinish`), when the file has any.
fn snapshot_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    summary: v8::Local<'s, v8::Object>,
) -> Option<SnapshotResult> {
    let k = v8::String::new(scope, "snapshot").unwrap();
    let o = summary
        .get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())?;
    let num = |key: &str| {
        let k = v8::String::new(scope, key).unwrap();
        o.get(scope, k.into())
            .and_then(|v| v.uint32_value(scope))
            .unwrap_or(0)
    };
    let (added, matched, unmatched, updated, unchecked) = (
        num("added"),
        num("matched"),
        num("unmatched"),
        num("updated"),
        num("unchecked"),
    );
    let k = v8::String::new(scope, "file_deleted").unwrap();
    let file_deleted = o
        .get(scope, k.into())
        .is_some_and(|v| v.boolean_value(scope));
    Some(SnapshotResult {
        added,
        matched,
        unmatched,
        updated,
        unchecked,
        unchecked_keys: string_array(scope, o, "unchecked_keys"),
        file_deleted,
    })
}

/// A test record's `location` (`[line, column]`), when it has one.
fn location_of<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
    let k = v8::String::new(scope, "location").unwrap();
    let arr = obj
        .get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())?;
    let at = |i: u32| {
        arr.get_index(scope, i)
            .and_then(|v| v.uint32_value(scope))
            .unwrap_or(0)
    };
    Some((at(0), at(1)))
}

/// The array `obj[key]` as strings (empty when it is not an array).
fn string_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
) -> Vec<String> {
    let k = v8::String::new(scope, key).unwrap();
    obj.get(scope, k.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
        .map(|arr| {
            (0..arr.length())
                .filter_map(|i| arr.get_index(scope, i))
                .map(|v| v.to_rust_string_lossy(scope))
                .collect()
        })
        .unwrap_or_default()
}

/// Call a no-arg global JS function and coerce its result to bool.
/// Call the global function `name` with no arguments: its number result (-1 when it is
/// missing, throws or returns no number).
fn call_global_number(scope: &mut v8::PinScope, global: v8::Local<v8::Object>, name: &str) -> f64 {
    let key = v8::String::new(scope, name).unwrap();
    let recv = v8::undefined(scope).into();
    global
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
        .and_then(|f| f.call(scope, recv, &[]))
        .and_then(|v| v.number_value(scope))
        .unwrap_or(-1.0)
}

fn call_global_bool(scope: &mut v8::PinScope, global: v8::Local<v8::Object>, name: &str) -> bool {
    let key = v8::String::new(scope, name).unwrap();
    let Some(f) = global
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
    else {
        return false;
    };
    let recv = v8::undefined(scope).into();
    match f.call(scope, recv, &[]) {
        Some(v) => v.boolean_value(scope),
        None => false,
    }
}

/// Panic recovery: after a caught panic the isolate is gone but its Global handles still
/// sit in the thread-local registry; dropping them would panic again ("disposed Isolate").
/// Replace the registry with a fresh one and LEAK the old handles (small, process-scoped).
pub fn forget_registry() {
    REGISTRY.with(|r| {
        let old = std::mem::take(&mut *r.borrow_mut());
        std::mem::forget(old);
    });
    // A panic may have left the reused isolate/context in a bad state — drop them so the next
    // file in this worker reinitializes from a fresh snapshot (correctness over cache reuse).
    REUSE_CTX.with(|c| {
        if let Some(g) = c.borrow_mut().take() {
            std::mem::forget(g);
        }
    });
    REUSE_ISO.with(|i| {
        if let Some(iso) = i.borrow_mut().take() {
            std::mem::forget(iso);
        }
    });
    DOM_INSTALLED.with(|d| d.set(false));
    SETUP_MOCKS.with(|s| s.borrow_mut().clear());
    SETUP_MOCK_SNAPSHOT.with(|s| s.borrow_mut().clear());
    REUSE_SETUP_ERROR.with(|e| e.borrow_mut().take());
}

fn clear_registry() {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.esm_by_path.clear();
        reg.cjs_synth_by_path.clear();
        reg.cjs_exports.clear();
        reg.real_exports.clear();
        reg.real_esm.clear();
        reg.cjs_export_names.clear();
        reg.path_by_hash.clear();
        reg.variant_by_hash.clear();
        reg.mocks.clear();
        reg.extra_exports.clear();
        reg.loading.clear();
        reg.lazy_stub_ns.clear();
        reg.import_edges.clear();
    });
}

/// The V8 flags a run applies: `TURBO_V8_FLAGS` when set (empty: none), else the defaults.
/// `--single-threaded`: each worker's isolate compiles and collects on its own thread. With an
/// isolate per core, V8's background compile and GC helpers (a platform pool as large as the
/// machine, shared by every isolate) only contend with the other workers: payroll's full run
/// takes about 9% less wall and CPU time, ui about the same, a run of a few dozen files at most
/// a tenth of a second more.
fn v8_flags(env: Option<&str>) -> String {
    env.unwrap_or("--single-threaded").to_string()
}

/// This run's V8 flags ([`v8_flags`] of `TURBO_V8_FLAGS`).
fn run_v8_flags() -> String {
    v8_flags(std::env::var("TURBO_V8_FLAGS").ok().as_deref())
}

/// Initialize V8 once for the process.
pub fn init_v8() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // V8 flags for an isolate per worker ([`v8_flags`]): overridable via TURBO_V8_FLAGS
        // (an empty value runs none) so the set is A/B-testable without a rebuild.
        let flags = run_v8_flags();
        if !flags.trim().is_empty() {
            v8::V8::set_flags_from_string(&flags);
        }
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
        // Build the framework snapshot once up front (not lazily on the first file),
        // so per-file setup timing reflects steady state, not the one-time build.
        let _ = framework_snapshot();
    });
}

#[cfg(test)]
mod env_pragma_tests {
    use super::file_env_pragma;

    #[test]
    fn parses_node_pragma() {
        assert_eq!(
            file_env_pragma("// @vitest-environment node\nimport x;").as_deref(),
            Some("node")
        );
    }

    #[test]
    fn parses_colon_and_jsdom() {
        assert_eq!(
            file_env_pragma("/* @vitest-environment: jsdom */").as_deref(),
            Some("jsdom")
        );
        assert_eq!(
            file_env_pragma("// @vitest-environment   happy-dom").as_deref(),
            Some("happy-dom")
        );
    }

    #[test]
    fn case_insensitive_value() {
        assert_eq!(
            file_env_pragma("// @vitest-environment Node").as_deref(),
            Some("node")
        );
    }

    #[test]
    fn none_when_absent() {
        assert_eq!(file_env_pragma("import { it } from 'vitest';"), None);
    }
}

// Unit tests for the runner's helpers, split by source region (kept in their own files so
// they are rustfmt-clean without reformatting this file).
#[cfg(all(test, unix))]
#[path = "runner_tests_a.rs"]
mod tests_a;
#[cfg(all(test, unix))]
#[path = "runner_tests_b.rs"]
mod tests_b;
#[cfg(all(test, unix))]
#[path = "runner_tests_jest.rs"]
mod tests_jest;
