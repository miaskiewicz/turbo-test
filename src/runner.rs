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

use crate::transform::maybe_transform;

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
    /// Genuine (unmocked) exports of every real module loaded via load_cjs, kept even after a
    /// mock overwrites cjs_exports — so vi.importActual/importOriginal returns the real module
    /// without re-running it (avoids dual instances of singletons like react-query).
    real_exports: HashMap<PathBuf, v8::Global<v8::Value>>,
    cjs_export_names: HashMap<PathBuf, Vec<String>>,
    path_by_hash: HashMap<i32, PathBuf>,
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
    static REUSE_ISO: RefCell<Option<v8::OwnedIsolate>> = RefCell::new(None);
    static REUSE_CTX: RefCell<Option<v8::Global<v8::Context>>> = RefCell::new(None);
    static DOM_INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// CommonJS-first module resolution (jest/node backend projects). Set once per run from the
    /// entry's project: a jest config with a node test environment resolves the `require` export
    /// condition (the build Node uses), so sequelize/tslib/lexical/etc. get their working CJS
    /// build instead of an ESM build that breaks once bundled to CJS. Off (ESM-first) for
    /// vitest/React projects so shared singletons (react/emotion/MUI) stay one instance.
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
    /// Snapshot (shallow clone) of each setup-mock's exports taken after the first file. A test
    /// file that re-mocks a setup path mutates the shared exports in place (to preserve identity
    /// for cached importers); restoring from this snapshot before the next file undoes that, so
    /// one file's analytics mock doesn't leak into the rest.
    static SETUP_MOCK_SNAPSHOT: RefCell<HashMap<PathBuf, v8::Global<v8::Value>>> = RefCell::new(HashMap::new());
}

/// Shallow-clone each setup-mock's current exports into SETUP_MOCK_SNAPSHOT (called once, after
/// the first file's setup runs).
fn snapshot_setup_mocks(scope: &mut v8::PinScope) {
    let paths: Vec<PathBuf> = SETUP_MOCKS.with(|s| s.borrow().iter().cloned().collect());
    let global = scope.get_current_context().global(scope);
    for p in paths {
        let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&p).cloned()) else { continue };
        let obj = v8::Local::new(scope, &g);
        let Some(o) = obj.to_object(scope) else { continue };
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

/// Restore each setup-mock's exports from its snapshot (called before every file after the
/// first), copying the saved props back onto the live (identity-preserved) exports object.
fn restore_setup_mocks(scope: &mut v8::PinScope) {
    let snaps: Vec<(PathBuf, v8::Global<v8::Value>)> =
        SETUP_MOCK_SNAPSHOT.with(|s| s.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let global = scope.get_current_context().global(scope);
    for (p, cg) in snaps {
        let Some(lg) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&p).cloned()) else { continue };
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

/// Mock-graph invalidation: when `target` is (re-)mocked, evict every app module that
/// (transitively) require()d it, so each re-imports the new mock instead of the stale version it
/// captured under a prior mock/real load. node_modules importers are NOT evicted (re-evaluating
/// them would re-trigger the emotion/MUI singleton accumulation that keeping modules warm avoids).
fn invalidate_importers(target: &Path) {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let mut stack = vec![target.to_path_buf()];
        let mut seen = std::collections::HashSet::new();
        let mut to_evict: Vec<PathBuf> = Vec::new();
        while let Some(m) = stack.pop() {
            let importers = reg.import_edges.get(&m).cloned().unwrap_or_default();
            for imp in importers {
                if imp.to_string_lossy().contains("/node_modules/") {
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

/// Per-file registry reset for the reuse path: drop app modules + ALL mocks, but KEEP genuinely
/// evaluated node_modules modules so the next file reuses them (the whole point). Any node_modules
/// path that was MOCKED in the prior file is evicted too, so the next file gets the real module.
fn reset_app_registry() {
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
        let test_mocked: Vec<PathBuf> =
            reg.mocks.keys().filter(|p| !setup_mocks.contains(*p)).cloned().collect();
        let evict = |p: &PathBuf| test_mocked.contains(p);
        reg.cjs_exports.retain(|p, _| !evict(p));
        reg.cjs_synth_by_path.retain(|p, _| !evict(p));
        reg.cjs_export_names.retain(|p, _| !evict(p));
        reg.real_exports.retain(|p, _| !evict(p));
        reg.esm_by_path.retain(|p, _| !evict(p));
        let live: std::collections::HashSet<PathBuf> =
            reg.cjs_exports.keys().chain(reg.esm_by_path.keys()).cloned().collect();
        reg.path_by_hash.retain(|_, p| live.contains(p));
        // keep setup mocks in the mock registry; drop test-file mocks
        reg.mocks.retain(|p, _| setup_mocks.contains(p));
        reg.extra_exports.clear();
        reg.loading.clear();
        reg.lazy_stub_ns.clear();
    });
}

/// Cached node-resolution Resolver (oxc_resolver) for bare specifiers / node_modules.
fn base_resolve_options(tsconfig: Option<PathBuf>, esm: bool) -> oxc_resolver::ResolveOptions {
    let strs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // ESM-FIRST for everything: resolving to a single (ESM) build per package keeps singletons
    // (react, @mui ThemeContext, @emotion cache) as ONE instance — CJS-first would give CJS
    // importers a separate CJS build → dual context. CJS-interop edges (e.g. recharts importing
    // eventemitter3's default) are handled by the __toESM function-default fix, not by switching
    // builds. (`esm` kept for signature compatibility.) (no "types" — .d.ts only.)
    //
    // NO "node" condition: turbo-test runs in a browser-like V8 (turbo-dom, esbuild
    // --platform=browser), and packages' `node` builds target the real Node runtime — they pull
    // node-only APIs and, worse, TOP-LEVEL AWAIT (e.g. lexical's *.node.mjs does
    // `const mod = await import(...)`), which esbuild cannot emit as --format=cjs. That bundle
    // then fails and falls back to a raw-.mjs CJS compile → "Unexpected token 'export'" load
    // errors. Dropping `node` makes the resolver pick the package's `default`/`browser` build,
    // which is the right target for this env and CJS-compiles cleanly. ("require" stays as a
    // last-resort condition so require-only packages still resolve.)
    let _ = esm;
    // A jest/node backend (NestJS, sequelize, ts-jest) is a CommonJS world: packages are
    // `require()`d, so their `require`/`node`/`default` export condition is the build that works
    // (sequelize's `lib/index.mjs` ESM build, picked by the `import` condition, loses
    // `DataTypes.ENUM` once bundled to CJS; `lib/index.js` is correct). Resolve CJS-first for such
    // projects. React/vitest projects keep ESM-first (one shared ESM instance of react/emotion/MUI
    // — switching them would risk dual-context). `node`/`default` cover packages that only ship
    // ESM, and a CJS-first bundle of an ESM file still works (esbuild ESM->CJS).
    // Condition matching is by the package's own key order against this SET — so to make `require`
    // win we must OMIT `import`/`module`/`browser`, not merely reorder. `node`/`default` still
    // cover packages without a `require` condition.
    let (conditions, mains): (&[&str], &[&str]) = if CJS_FIRST_RESOLUTION.with(|c| c.get()) {
        (&["require", "node", "default"], &["main", "module", "browser"])
    } else {
        (&["import", "module", "browser", "default", "require"], &["module", "browser", "main"])
    };
    oxc_resolver::ResolveOptions {
        extensions: strs(&[".ts", ".tsx", ".mjs", ".js", ".jsx", ".cjs", ".json", ".node"]),
        condition_names: strs(conditions),
        main_fields: strs(mains),
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
    let mut d = Some(dir);
    while let Some(cur) = d {
        let tc = cur.join("tsconfig.json");
        if tc.is_file() {
            return Some(tc);
        }
        if cur.join("package.json").is_file() && cur.file_name().map(|n| n == "node_modules").unwrap_or(false) {
            break;
        }
        d = cur.parent();
    }
    None
}

/// Is `abs` an ES module (so its imports resolve with the "import" condition)? .mjs/.ts/.tsx are
/// ESM; .cjs is CJS; .js follows the nearest package.json "type".
fn is_esm_module(abs: &Path) -> bool {
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

/// A resolver configured for the nearest tsconfig of `from_dir` + the importer's module kind
/// (ESM vs CJS conditions), cached per (tsconfig, esm).
fn resolver_for(from_dir: &Path, esm: bool) -> std::rc::Rc<oxc_resolver::Resolver> {
    thread_local! {
        static RESOLVERS: RefCell<HashMap<(Option<PathBuf>, bool, bool), std::rc::Rc<oxc_resolver::Resolver>>> =
            RefCell::new(HashMap::new());
    }
    // CJS-first mode is part of the key so a flip rebuilds the resolver (it changes conditions).
    let key = (nearest_tsconfig(from_dir), esm, CJS_FIRST_RESOLUTION.with(|c| c.get()));
    RESOLVERS.with(|m| {
        if let Some(r) = m.borrow().get(&key) {
            return r.clone();
        }
        let r = std::rc::Rc::new(oxc_resolver::Resolver::new(base_resolve_options(key.0.clone(), esm)));
        m.borrow_mut().insert(key, r.clone());
        r
    })
}

/// Decide CJS-first resolution for a project: true when the nearest config is a jest config with a
/// node (non-DOM) test environment and there's no vitest config — i.e. a CommonJS backend. Walks
/// up from `entry`; a vitest config short-circuits to ESM-first.
fn cjs_first_project(entry: &Path) -> bool {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        for v in ["vitest.config.ts", "vitest.config.mts", "vitest.config.js", "vitest.config.mjs", "vite.config.ts", "vite.config.js"] {
            if d.join(v).is_file() {
                return false; // vitest project → ESM-first
            }
        }
        let jest_cfg = ["jest.config.js", "jest.config.cjs", "jest.config.mjs", "jest.config.ts", "jest.config.json"]
            .iter()
            .map(|n| d.join(n))
            .find(|p| p.is_file());
        let pkg_jest = std::fs::read_to_string(d.join("package.json")).ok()
            .filter(|s| find_config_key(s, "jest").is_some());
        if let Some(text) = jest_cfg.and_then(|p| std::fs::read_to_string(p).ok()).or(pkg_jest) {
            // node test env (jest's default) unless it explicitly asks for a DOM env.
            let env = config_string_value(&text, "testEnvironment");
            return !matches!(env.as_deref(), Some("jsdom") | Some("happy-dom") | Some("jest-environment-jsdom"));
        }
        dir = d.parent();
    }
    false
}

/// Kind by extension only (unambiguous extensions).
fn kind_of(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()) {
        Some("cjs") => Kind::Cjs,
        _ => Kind::Esm,
    }
}

/// Nearest package.json `"type"` for a file (Node's module-determination rule).
fn nearest_pkg_type(path: &Path) -> Option<&'static str> {
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
/// follows the nearest package.json `"type"` (Node's rule), falling back to a syntax sniff.
fn detect_kind(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()) {
        Some("cjs") => Kind::Cjs,
        Some("mjs" | "mts" | "ts" | "tsx" | "jsx") => Kind::Esm,
        _ => match nearest_pkg_type(path) {
            Some("module") => Kind::Esm,
            Some("commonjs") => Kind::Cjs,
            _ => {
                let src = std::fs::read_to_string(path).unwrap_or_default();
                let esm = src.contains("import ")
                    || src.contains("import{")
                    || src.contains("export ")
                    || src.contains("export{")
                    || src.contains("export*")
                    || src.contains("export default");
                let cjs = src.contains("module.exports")
                    || src.contains("exports.")
                    || src.contains("exports[")
                    || src.contains("require(");
                if cjs && !esm {
                    Kind::Cjs
                } else {
                    Kind::Esm
                }
            }
        },
    }
}

pub fn resolve_spec(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    resolve_spec_as(spec, from_dir, true)
}

/// Resolve with the importer's module kind (`esm`) selecting ESM vs CJS export conditions.
pub fn resolve_spec_as(spec: &str, from_dir: &Path, esm: bool) -> Option<PathBuf> {
    // A `node_modules/<pkg>/...` import (some tests `await import('node_modules/x/dist/y')`)
    // is really a bare specifier — strip the prefix so oxc resolves it from node_modules.
    let spec = spec.strip_prefix("node_modules/").unwrap_or(spec);
    // Unified Node resolution (relative + bare) via oxc_resolver — handles package.json
    // main/exports/imports, conditions, index files, and extensions uniformly. This is
    // what makes deep node_modules graphs (@mui internal "./styles" etc.) resolve.
    if let Ok(res) = resolver_for(from_dir, esm).resolve(from_dir, spec) {
        if let Ok(c) = std::fs::canonicalize(res.path()) {
            return Some(c);
        }
    }
    // fallback: manual probing for local files oxc may skip
    if spec.starts_with('.') || spec.starts_with('/') {
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
                return std::fs::canonicalize(c).ok();
            }
        }
    }
    None
}

/// Non-JS assets that Vite/Vitest stub: importing them must not crash the graph.
fn is_asset(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(
            "css" | "scss" | "sass" | "less" | "styl" | "svg" | "png" | "jpg" | "jpeg" | "gif"
                | "webp" | "avif" | "ico" | "woff" | "woff2" | "ttf" | "eot" | "otf" | "mp4"
                | "webm" | "wav" | "mp3" | "txt" | "md"
        )
    )
}

fn module_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::String>,
) -> v8::ScriptOrigin<'s> {
    v8::ScriptOrigin::new(scope, name.into(), 0, 0, false, 123, None, false, false, true, None)
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
    (CACHE_HITS.load(Ordering::Relaxed), CACHE_MISSES.load(Ordering::Relaxed))
}

const TRANSFORM_VERSION: &str = "oxc-0.134-v1";

fn cache_dir() -> PathBuf {
    let d = std::env::temp_dir().join("turbo-test-cache");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Content-addressed key: hash(source + extension + transform-tool version).
/// A change in any input misses the cache; identical inputs hit across runs/workers.
fn cache_key(abs: &Path, src: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    abs.extension().and_then(|e| e.to_str()).unwrap_or("").hash(&mut h);
    TRANSFORM_VERSION.hash(&mut h);
    h.finish()
}

/// Nearest project root containing esbuild (for dep bundling).
fn project_root(file: &Path) -> Option<PathBuf> {
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir.join("node_modules/.bin/esbuild").exists() {
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
/// Extract vi.mock("spec", ...) / vi.doMock(...) string-literal specifiers from a file.
fn mock_specifiers(file: &Path) -> Vec<String> {
    let Ok(src) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for kw in ["vi.mock(", "vi.doMock(", "jest.mock(", "jest.doMock("] {
        let mut i = 0;
        while let Some(p) = src[i..].find(kw) {
            let start = i + p + kw.len();
            i = start;
            let rest = src[start..].trim_start();
            let mut ch = rest.chars();
            if let Some(q) = ch.next() {
                if q == '"' || q == '\'' || q == '`' {
                    if let Some(end) = rest[1..].find(q) {
                        let spec = rest[1..1 + end].to_string();
                        if !out.contains(&spec) {
                            out.push(spec);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Bundle a test/setup file. `externals` are vi.mock specifiers kept OUT of the bundle so the
/// native loader (+ mock registry) intercepts them; after bundling they are rewritten from
/// their (file-relative) form to the resolved ABSOLUTE path so they resolve correctly from
/// the bundle's cache location and match the mock registered under the same absolute path.
fn esbuild_bundle(file: &Path, externals: &[String]) -> Option<PathBuf> {
    esbuild_bundle_full(file, externals, &std::collections::HashMap::new(), None)
}

/// last path segment of a specifier (the module "basename"), e.g. "../../x/analytics" -> "analytics".
fn spec_basename(spec: &str) -> String {
    spec.trim_end_matches('/').rsplit('/').next().unwrap_or(spec).to_string()
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
    let esbuild = root.join("node_modules/.bin/esbuild");
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
    "esb-v4-react-ext".hash(&mut h);
    mr_enabled().hash(&mut h);
    let out = cache_dir().join(format!("esb-{:016x}.mjs", h.finish()));
    // Record the source dir so resolve_callback can resolve this bundle's externalized bare
    // imports (the cache file has no node_modules of its own).
    if let Some(src_dir) = file.parent() {
        REGISTRY.with(|r| r.borrow_mut().bundle_src_dir.insert(out.clone(), src_dir.to_path_buf()));
    }
    if out.exists() {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(out);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    // tsconfig (for path aliases like "@/*") — esbuild resolves them when pointed at it.
    let mut tsconfig = None;
    {
        let mut d = file.parent();
        while let Some(dir) = d {
            let tc = dir.join("tsconfig.json");
            if tc.is_file() {
                tsconfig = Some(tc);
                break;
            }
            if dir == root {
                break;
            }
            d = dir.parent();
        }
    }
    let mut cmd = std::process::Command::new(&esbuild);
    cmd.current_dir(&root).arg(file).args([
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
    // Under the module-runner, this ESM bundle (setup files) must NOT inline node_modules
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
    cmd.arg(format!("--outfile={}", out.display()))
        .stdout(std::process::Stdio::null());
    if std::env::var("TURBO_ESBUILD_DEBUG").is_err() {
        cmd.stderr(std::process::Stdio::null());
    }
    let status = cmd.status().ok()?;
    if !(status.success() && out.exists()) {
        return None;
    }
    // Rewrite every externalized mock import to the single absolute mock target for its
    // basename. All depth-variants of a relative mock specifier ("../analytics",
    // "../../../analytics", ...) point at the same module, so they map to one abs path that
    // matches the mock registered under that path — regardless of which nested module wrote
    // the import (whose depth differs from the entry's).
    if !externals.is_empty() && !rewrite_map.is_empty() {
        if let Ok(mut text) = std::fs::read_to_string(&out) {
            let mut changed = false;
            for spec in externals {
                if let Some(abs) = rewrite_map.get(&spec_basename(spec)) {
                    let abs_s = abs.to_string_lossy();
                    for q in ['"', '\''] {
                        let from = format!("{q}{spec}{q}");
                        let to = format!("{q}{abs_s}{q}");
                        if text.contains(&from) {
                            text = text.replace(&from, &to);
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                let _ = std::fs::write(&out, &text);
            }
        }
    }
    Some(out)
}

/// Nearest dir containing node_modules/@miaskiewicz/turbo-dom (the DOM environment).
fn turbodom_root(file: &Path) -> Option<PathBuf> {
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir.join("node_modules/@miaskiewicz/turbo-dom/src/environment/install.mjs").is_file() {
            return Some(dir.to_path_buf());
        }
        d = dir.parent();
    }
    None
}

/// A tiny ESM bootstrap that installs turbo-dom's window/document onto globalThis. Lives in
/// the cache dir and imports install.mjs by absolute path (so node_modules resolution + our
/// node-builtin shims + the napi-loaded .node parser all kick in via the native loader).
fn dom_bootstrap(root: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    let out = cache_dir().join(format!("dom-boot-{:016x}.mjs", h.finish()));
    // After installGlobals, shim CSSOM that emotion/MUI need but turbo-dom doesn't expose:
    // a working `.sheet` (insertRule/cssRules) on <style> elements, and document.styleSheets.
    // emotion's sheetForTag reads tag.sheet.cssRules.length → crashes without this.
    let src = format!(
        "import {{ installGlobals }} from '{root}/node_modules/@miaskiewicz/turbo-dom/src/environment/install.mjs';\n\
         globalThis.__turboDomEnv = installGlobals(globalThis, {{}});\n\
         (function () {{\n\
           const sheets = [];\n\
           const mkSheet = (el) => {{ const rules = []; return {{ ownerNode: el, cssRules: rules, get rules() {{ return rules; }},\n\
             insertRule(rule, index) {{ const i = index == null ? rules.length : index; rules.splice(i, 0, {{ cssText: String(rule), selectorText: '' }}); return i; }},\n\
             deleteRule(i) {{ rules.splice(i, 1); }}, replaceSync() {{}}, replace() {{ return Promise.resolve(); }} }}; }};\n\
           if (typeof document !== 'undefined' && document.createElement) {{\n\
             const orig = document.createElement.bind(document);\n\
             document.createElement = function (tag) {{ const el = orig(tag); try {{ if (String(tag).toLowerCase() === 'style' && !el.sheet) {{ const s = mkSheet(el); Object.defineProperty(el, 'sheet', {{ configurable: true, get: () => s }}); sheets.push(s); }} }} catch (e) {{}} return el; }};\n\
             if (!document.styleSheets) {{ try {{ Object.defineProperty(document, 'styleSheets', {{ configurable: true, get: () => sheets }}); }} catch (e) {{}} }}\n\
           }}\n\
         }})();\n",
        root = root.display()
    );
    if std::fs::read_to_string(&out).ok().as_deref() != Some(src.as_str()) {
        std::fs::write(&out, &src).ok()?;
    }
    Some(out)
}

/// Whether a file needs a DOM environment (so we don't impose one on pure-logic tests,
/// which keeps their clean globals — mirrors vitest's per-file environment).
fn needs_dom(file: &Path) -> bool {
    if matches!(file.extension().and_then(|e| e.to_str()), Some("tsx" | "jsx")) {
        return true;
    }
    let s = std::fs::read_to_string(file).unwrap_or_default();
    s.contains("@testing-library")
        || s.contains("react-dom")
        || s.contains("ReactDOM")
        || s.contains("document")
        || s.contains("window.")
        || s.contains("getComputedStyle")
        || s.contains("happy-dom")
        || s.contains("jsdom")
        || s.contains("@vitest-environment")
}

/// Whether the project asks for non-isolated runs (`isolate: false`) in any of its vitest
/// configs near `entry`. turbo-test then reuses one isolate per worker (vitest isolate:false
/// semantics) — node_modules barrels evaluate once, not per file. The env var TURBO_REUSE_ISOLATE
/// forces it on regardless; TURBO_NO_REUSE forces it off.
fn vitest_isolate_false(entry: &Path) -> bool {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        // any vitest config variant the project uses (dev/ci/coverage/shard commonly set isolate)
        for cfg in [
            "vitest.config.ts", "vitest.config.mts", "vitest.config.js",
            "vitest.config.dev.ts", "vitest.config.ci.ts", "vite.config.ts",
        ] {
            if let Ok(s) = std::fs::read_to_string(d.join(cfg)) {
                // strip `//` line comments first — a doc comment mentioning "isolate: false"
                // must not be mistaken for the real setting.
                let code: String = s
                    .lines()
                    .map(|l| l.split("//").next().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");
                let flat: String = code.chars().filter(|c| !c.is_whitespace()).collect();
                if flat.contains("isolate:false") {
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
/// (`key:`), skipping mentions inside `//` comments or prose (e.g. a doc comment that names
/// `setupFiles`). Returns the byte offset of the key name.
fn find_config_key(s: &str, key: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(i) = s[search..].find(key) {
        let at = search + i;
        search = at + key.len();
        // must be immediately followed (after optional whitespace) by ':'
        if !s[at + key.len()..].trim_start().starts_with(':') {
            continue;
        }
        // skip if this line is a `//` comment
        let line_start = s[..at].rfind('\n').map(|n| n + 1).unwrap_or(0);
        if s[line_start..at].trim_start().starts_with("//") {
            continue;
        }
        return Some(at);
    }
    None
}

fn vitest_setup_files(entry: &Path) -> Vec<PathBuf> {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        for cfg in ["vitest.config.ts", "vitest.config.mts", "vite.config.ts", "vitest.config.js"] {
            let p = d.join(cfg);
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Some(start) = find_config_key(&s, "setupFiles") {
                    // grab the [...] (or single quoted) right after setupFiles
                    let tail = &s[start..];
                    if let Some(lb) = tail.find('[') {
                        if let Some(rb) = tail[lb..].find(']') {
                            let inner = &tail[lb + 1..lb + rb];
                            let mut out = Vec::new();
                            for part in inner.split(',') {
                                let q = part.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`');
                                if q.is_empty() {
                                    continue;
                                }
                                if let Some(abs) = resolve_spec(q, d) {
                                    out.push(abs);
                                }
                            }
                            if !out.is_empty() {
                                return out;
                            }
                        }
                    }
                }
                return Vec::new(); // config found, no setupFiles
            }
        }
        // No vitest config in this dir — try a jest config (drop-in for jest projects).
        if let Some(found) = jest_setup_files(d) {
            return found;
        }
        dir = d.parent();
    }
    Vec::new()
}

/// Jest config parity: read `setupFiles` + `setupFilesAfterEnv` from a jest config in `dir`
/// (`jest.config.{js,cjs,mjs,ts,json}` or a `"jest"` block in package.json), resolving Jest's
/// `<rootDir>` token (rootDir defaults to the config dir). Returns None if no jest config here
/// (so the caller keeps walking up); Some(possibly-empty) once a jest config is found.
fn jest_setup_files(dir: &Path) -> Option<Vec<PathBuf>> {
    let names = [
        "jest.config.js", "jest.config.cjs", "jest.config.mjs",
        "jest.config.ts", "jest.config.json",
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
        if let Ok(pkg) = std::fs::read_to_string(dir.join("package.json")) {
            if let Some(start) = find_config_key(&pkg, "jest") {
                if pkg[start..].trim_start().starts_with("jest")
                    || pkg[start..].contains('{')
                {
                    text = Some(pkg);
                }
            }
        }
    }
    let s = text?;
    // rootDir (default = config dir). Jest paths in setupFiles use `<rootDir>`.
    let root_dir = config_string_value(&s, "rootDir")
        .map(|r| normalize_path(&dir.join(&r)))
        .unwrap_or_else(|| dir.to_path_buf());
    let mut out = Vec::new();
    for key in ["setupFiles", "setupFilesAfterEnv"] {
        if let Some(start) = find_config_key(&s, key) {
            let tail = &s[start..];
            if let Some(lb) = tail.find('[') {
                if let Some(rb) = tail[lb..].find(']') {
                    for part in tail[lb + 1..lb + rb].split(',') {
                        let q = part.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`');
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

/// Normalize a path lexically (resolve `..`/`.`) without touching the filesystem.
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Run a setup module (bundled, with mock specifiers externalized) in the current context,
/// then drain any vi.mock() calls it made (resolved relative to the setup file's dir).
fn run_setup_file(
    scope: &mut v8::PinScope,
    file: &Path,
    externals: &[String],
    rewrite_map: &std::collections::HashMap<String, PathBuf>,
) {
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
    if let Some(module) = load_graph(scope, &target) {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        if module.instantiate_module(tc, resolve_callback) == Some(true) {
            let _ = module.evaluate(tc);
            tc.perform_microtask_checkpoint();
        }
    }
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, file.parent().unwrap_or(Path::new(".")), true);
}

/// Extract `const/let/var ... = vi.hoisted(...)` declarations (full statements) so they can
/// be replayed in the mock prepass — mock factories commonly close over hoisted vars.
fn extract_hoisted(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while let Some(p) = src.get(i..).and_then(|s| s.find("vi.hoisted(")) {
        let at = i + p;
        i = at + "vi.hoisted(".len();
        // statement start: nearest preceding `const `/`let `/`var ` on the way back
        let head = src.get(..at).unwrap_or("");
        let start = ["const ", "let ", "var "]
            .iter()
            .filter_map(|kw| head.rfind(kw))
            .max();
        let Some(start) = start else { continue };
        // balance parens from vi.hoisted( to its close, then include up to ';'
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
        // swallow a trailing semicolon if present
        while k < bytes.len() && (bytes[k] as char).is_whitespace() {
            k += 1;
        }
        if k < bytes.len() && bytes[k] as char == ';' {
            k += 1;
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
fn run_entry_mocks(scope: &mut v8::PinScope, entry: &Path) {
    let src = std::fs::read_to_string(entry).unwrap_or_default();
    let mocks = extract_mocks(&src);
    if mocks.is_empty() {
        return;
    }
    let dir = entry.parent().unwrap_or(Path::new("."));
    let Some(root) = project_root(entry) else {
        // No esbuild project (e.g. standalone fixtures): register self-contained factories
        // directly (no JSX/transform available here).
        for (spec, factory) in &mocks {
            if let Some(abs) = resolve_spec(spec, dir) {
                register_mock(scope, &abs, factory);
            }
        }
        return;
    };
    // React is imported here too: mock factories commonly reference the test's module-scope
    // `React` (e.g. `React.useEffect` inside a returned component) which isn't in this prepass.
    let mut content = String::from("import { vi } from 'vitest';\nimport React from 'react';\n");
    // vi.hoisted() declarations that the mock factories reference must exist in the prepass.
    for decl in extract_hoisted(&src) {
        content.push_str(&decl);
        content.push('\n');
    }
    for (spec, factory) in &mocks {
        content.push_str(&format!("vi.mock({:?}, {});\n", spec, factory));
    }
    // Route module-`let`s the factories close over through globalThis.__ms so they share the
    // SAME binding as the entry module (which esbuild_transform_cjs rewrites identically).
    let shared = shared_mock_lets(&src);
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
    {
        let global = scope.get_current_context().global(scope);
        if let Some(key) = v8::String::new(scope, "__pendingMocks") {
            let empty = v8::Array::new(scope, 0);
            global.set(scope, key.into(), empty.into());
        }
    }
    if mr_enabled() {
        // Module-runner: run the mock factories IN THE CURRENT realm (CJS transform + wrapper)
        // so their require('react')/jsx-runtime resolve to the SAME shared react the test uses
        // — React-component mocks then render correctly (no dual-react). Runs before the entry.
        // Guard the whole prepass: an async factory's `await import('<self>')` resolves during the
        // microtask checkpoint below, re-entering load_cjs for the real module. Its nested drain
        // would clear the pending-mock queue and register the still-pending factory promise as an
        // empty mock (so the real module wins and named imports never rebind to the spies). The
        // flag makes load_cjs leave the queue alone until the outer drain runs here.
        let prev_resolving = RESOLVING_MOCKS.with(|f| f.replace(true));
        if let Some(raw) = esbuild_transform_cjs(&p, false) {
            let wrapped = format!(
                "(function (exports, module, require, __filename, __dirname) {{\n{raw}\n}})"
            );
            if let Some(code) = v8::String::new(scope, &wrapped) {
                let tc = std::pin::pin!(v8::TryCatch::new(scope));
                let tc = &mut tc.init();
                if let Some(script) = v8::Script::compile(tc, code, None) {
                    if let Some(wf) = script.run(tc).and_then(|v| v8::Local::<v8::Function>::try_from(v).ok()) {
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
                        if let Some(code) = v8::String::new(tc, "globalThis.__hoistedCache = []; globalThis.__hoistedIdx = 0;") {
                            if let Some(s) = v8::Script::compile(tc, code, None) { s.run(tc); }
                        }
                        if let Some(req) = v8::String::new(tc, "__mkRequire")
                            .and_then(|k| g.get(tc, k.into()))
                            .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
                        {
                            let undef = v8::undefined(tc).into();
                            if let Some(require) = req.call(tc, undef, &[dir_str.into(), v8::Boolean::new(tc, true).into()]) {
                                let fname = v8::String::new(tc, &entry.to_string_lossy()).unwrap();
                                wf.call(tc, undef, &[exports_obj.into(), module_obj.into(), require, fname.into(), dir_str.into()]);
                                tc.perform_microtask_checkpoint();
                            }
                        }
                    }
                }
            }
        }
        resolve_pending_mocks(scope);
        drain_pending_mocks(scope, entry.parent().unwrap_or(Path::new(".")), false);
        RESOLVING_MOCKS.with(|f| f.set(prev_resolving));
        return;
    }
    let bundled = esbuild_bundle(&p, &[]).unwrap_or_else(|| p.clone());
    if let Some(module) = load_graph(scope, &bundled) {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        if module.instantiate_module(tc, resolve_callback) == Some(true) {
            let _ = module.evaluate(tc);
            tc.perform_microtask_checkpoint();
        }
    }
    resolve_pending_mocks(scope);
    drain_pending_mocks(scope, entry.parent().unwrap_or(Path::new(".")), false);
}

/// Set up the DOM environment in the current context (best-effort): load + run the turbo-dom
/// bootstrap so document/window exist before the test evaluates.
fn setup_dom(scope: &mut v8::PinScope, entry: &Path) {
    let Some(root) = turbodom_root(entry) else { return };
    let Some(boot) = dom_bootstrap(&root) else { return };
    let Some(module) = load_graph(scope, &boot) else {
        eprintln!("dom bootstrap load failed");
        return;
    };
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    let inst = module.instantiate_module(tc, resolve_callback);
    if inst != Some(true) {
        eprintln!("dom bootstrap instantiate failed: {:?}", inst);
        if tc.has_caught() {
            eprintln!("  exc: {}", tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_default());
        }
        return;
    }
    let _ = module.evaluate(tc);
    tc.perform_microtask_checkpoint();
    // turbo-dom's installGlobals clobbered Blob/File/FileReader with SoA-backed versions that
    // read zero bytes — restore ours so file reads return real content.
    if let Some(code) = v8::String::new(tc, "if (globalThis.__ttRestoreFileApis) globalThis.__ttRestoreFileApis();") {
        if let Some(s) = v8::Script::compile(tc, code, None) { s.run(tc); }
    }
    if tc.has_caught() {
        eprintln!("dom bootstrap threw: {}", tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_default());
    } else if module.get_status() == v8::ModuleStatus::Errored {
        eprintln!("dom bootstrap errored: {}", module.get_exception().to_rust_string_lossy(tc));
    } else {
        eprintln!("dom bootstrap OK, status={:?}", module.get_status());
    }
}

/// Transform a TS file to **ESM** JS using the PROJECT'S OWN TypeScript (`ts.transpileModule`),
/// lowering decorators + `emitDecoratorMetadata` with exact ts-jest parity. Unlike esbuild (no
/// metadata) and oxc 0.134 (emits `Object` for type-alias / nullable types), tsc resolves local
/// type aliases (`type Percentage = number` -> `Number`) so NestJS/Mongoose decorators get the
/// right `design:type`. Module syntax is kept as ESM so the caller's esbuild ESM->CJS pass gives
/// the same `var import_X = require(...)` shape the rest of the pipeline (vi.mock hoisting) needs.
/// Returns None if the project ships no `typescript` (caller falls back to oxc).
fn tsc_transform_esm(file: &Path, root: &Path) -> Option<String> {
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
const src = fs.readFileSync(file, 'utf8');
const isTsx = file.endsWith('.tsx');
const out = ts.transpileModule(src, {
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
    let out = std::process::Command::new("node")
        .current_dir(root)
        .args(["-e", SCRIPT, &ts_dir.to_string_lossy(), &file.to_string_lossy()])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
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
    let esbuild = root.join("node_modules/.bin/esbuild");
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
    let Some(tc) = nearest_tsconfig(file.parent().unwrap_or(Path::new("."))) else { return false };
    if let Some(v) = CACHE.with(|c| c.borrow().get(&tc).copied()) {
        return v;
    }
    // Strip `//` line comments before scanning so a commented mention doesn't false-positive.
    let on = std::fs::read_to_string(&tc)
        .map(|s| {
            let stripped: String = s.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
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
        ch.next() == Some('@') && ch.next().map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$').unwrap_or(false)
    })
}

/// Module-runner mode: transform a single APP module to CJS (no bundle) so imports become
/// live `require(...).name` property access (mutable → spyOn/vi.mock work), and post-process
/// esbuild's export getters to be CONFIGURABLE (so spyOn can redefine them). Cached.
fn esbuild_transform_cjs(file: &Path, prefer_metadata: bool) -> Option<String> {
    let root = project_root(file)?;
    let esbuild = root.join("node_modules/.bin/esbuild");
    use std::hash::{Hash, Hasher};
    let raw = std::fs::read_to_string(file).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    file.extension().and_then(|e| e.to_str()).unwrap_or("").hash(&mut h);
    "mr-cjs-v1".hash(&mut h);
    // esbuild can't emit decorator metadata. By default we DON'T pay that cost — only when a
    // decorator file actually threw at load (Mongoose @Prop / Sequelize @Column read design:type
    // at class-definition) does the loader retry with `prefer_metadata`, routing it through the
    // project's TypeScript (ts.transpileModule, exact ts-jest parity) so the metadata is emitted.
    // Retry-on-failure keeps the common case on fast esbuild and never regresses files that load
    // fine without metadata. Keyed separately so the two transforms never collide in cache.
    let use_meta = prefer_metadata && decorator_metadata_enabled(file) && file_has_decorator(&raw);
    use_meta.hash(&mut h);
    // Coverage emits an inline source map (needed to remap V8 byte ranges → original lines), so
    // the output differs — key it separately. Only perturb the key when coverage is ON, so the
    // normal (no-map) cache keys are byte-identical to before → zero cache churn on default runs.
    let cov = crate::coverage::enabled();
    if cov {
        "cov-inline-map".hash(&mut h);
    }
    let cache = cache_dir().join(format!("mr-{:016x}.cjs", h.finish()));
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    if use_meta {
        // Primary: the project's own TypeScript (ts.transpileModule) → exact ts-jest parity for
        // emitDecoratorMetadata (resolves local type aliases, nullable types, etc.). Fallback:
        // oxc lowers decorators+metadata (ESM) then esbuild converts ESM->CJS — used when the
        // project ships no `typescript`. oxc can panic on some inputs, so it's caught; either way
        // we fall through to the plain esbuild transform (metadata-less but loads) on failure.
        // Get ESM-with-metadata from tsc (preferred) or oxc (fallback), then a single esbuild
        // ESM->CJS pass so every path yields the same esbuild-shaped CJS the loader/hoisting want.
        let meta_esm = tsc_transform_esm(file, &root).or_else(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::transform::transform_decorators_with_metadata(file, &raw)
            }))
            .ok()
            .and_then(|r| r.ok())
        });
        if let Some(cjs) = meta_esm.and_then(|esm| esbuild_format_cjs(&root, &esm)) {
            let mut code = postprocess_mr_cjs(cjs);
            code = hoist_mock_setup(&code);
            let shared = shared_mock_lets(&raw);
            if !shared.is_empty() {
                code = rewrite_shared_lets(&code, &shared);
            }
            write_atomic(&cache, &code);
            return Some(code);
        }
        // fall through to the plain esbuild transform below (metadata-less, but loads)
    }
    let mut tsconfig = None;
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir.join("tsconfig.json").is_file() {
            tsconfig = Some(dir.join("tsconfig.json"));
            break;
        }
        if dir == root { break; }
        d = dir.parent();
    }
    let mut cmd = std::process::Command::new(&esbuild);
    cmd.current_dir(&root).arg(file).args([
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
    let out = cmd.stderr(std::process::Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut code = postprocess_mr_cjs(String::from_utf8(out.stdout).ok()?);
    // vitest-style vi.mock hoisting: move consumer requires below the vi.mock setup so the mock
    // is registered (immediately, in this module's scope) before the mocked module loads.
    code = hoist_mock_setup(&code);
    // Module-`let`s that this file's vi.mock() factory closes over (writes from inside the mock,
    // read by the test) must be SHARED between the mock-prepass scope and this module's scope.
    // vitest shares them via one module scope; we can't (the factory runs in the prepass), so
    // route those lets through a shared global. Same rewrite is applied to the prepass factory.
    let shared = shared_mock_lets(&raw);
    if !shared.is_empty() {
        code = rewrite_shared_lets(&code, &shared);
    }
    write_atomic(&cache, &code);
    Some(code)
}

/// vitest-style vi.mock hoisting on esbuild CJS output: esbuild emits all `require(...)` at the
/// top, then the body (module decls + vi.mock + tests). A vi.mock must be registered before the
/// mocked module loads, so we move the requires NOT referenced by the mock-setup region (the
/// "consumers" — incl. the modules being mocked) to AFTER that region. The setup's own decls run
/// in-module-scope, so the factory shares the test's bindings (push, mockUseAuthInfo, etc.).
fn hoist_mock_setup(code: &str) -> String {
    if !code.contains(".mock(") {
        return code.to_string();
    }
    let lines: Vec<&str> = code.lines().collect();
    let is_req = |l: &str| l.starts_with("var import_") && l.contains("require(");
    // contiguous require block
    let Some(req_start) = lines.iter().position(|l| is_req(l)) else {
        return code.to_string();
    };
    let mut req_end = req_start;
    while req_end + 1 < lines.len() && is_req(lines[req_end + 1]) {
        req_end += 1;
    }
    // end of the last top-level `.mock(` statement (balance parens from the call to its close + ;)
    let Some(last_mock) = lines.iter().rposition(|l| l.contains(".mock(")) else {
        return code.to_string();
    };
    if last_mock <= req_end {
        return code.to_string();
    }
    let mut depth = 0i32;
    let mut started = false;
    let mut region_end = last_mock;
    'outer: for (i, l) in lines.iter().enumerate().skip(last_mock) {
        for ch in l.chars() {
            match ch {
                '(' => { depth += 1; started = true; }
                ')' => depth -= 1,
                _ => {}
            }
        }
        if started && depth <= 0 {
            region_end = i;
            break 'outer;
        }
    }
    let setup = &lines[req_end + 1..=region_end];
    // Split the setup region into: pre-mock decls (factory-referenced vars declared BEFORE the
    // first vi.mock — must stay before, the factory closes over them), the vi.mock statements
    // themselves, and post-mock decls (e.g. `const x = vi.mocked(import_y.z)` interleaved with /
    // after the mocks — must run AFTER the requires are re-bound to the mocks). esbuild emits
    // factory-dep decls before the first mock and capture decls after, so split at the 1st mock.
    let first_mock_rel = setup.iter().position(|l| l.contains(".mock(")).unwrap_or(0);
    let pre_setup: Vec<&str> = setup[..first_mock_rel].to_vec();
    // walk the tail, separating full `.mock(...)` statements from interleaved decls
    let mut mock_stmts: Vec<&str> = Vec::new();
    let mut post_decls: Vec<&str> = Vec::new();
    let tail = &setup[first_mock_rel..];
    let mut k = 0usize;
    while k < tail.len() {
        if tail[k].contains(".mock(") {
            let mut depth = 0i32;
            let mut started = false;
            let start = k;
            while k < tail.len() {
                for ch in tail[k].chars() {
                    match ch {
                        '(' => { depth += 1; started = true; }
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                k += 1;
                if started && depth <= 0 { break; }
            }
            mock_stmts.extend_from_slice(&tail[start..k]);
        } else {
            post_decls.push(tail[k]);
            k += 1;
        }
    }
    // A require stays BEFORE the vi.mocks if its value is needed at LOAD time — by a mock factory
    // or by top-level code (e.g. `class X extends React.Component`). A require used only inside
    // test/hook callbacks runs later, so it can be a consumer that loads AFTER the mocks (this is
    // what makes `vi.mock` placed below the tests still intercept). We test references against the
    // setup region with test-callback bodies stripped, plus the factory bodies explicitly.
    let setup_text_full = setup.join("\n");
    let setup_text = format!("{}\n{}", strip_test_blocks(&setup_text_full), mock_stmts.join("\n"));
    let mut needed = Vec::new();
    let mut consumers = Vec::new();
    for &l in &lines[req_start..=req_end] {
        // var import_NAME = ...
        let name = l.strip_prefix("var ").unwrap_or(l);
        let name: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
        if !name.is_empty() && contains_word(&setup_text, &name) {
            needed.push(l);
        } else {
            consumers.push(l);
        }
    }
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
        for &l in &lines[req_start..=req_end] {
            if let Some(spec) = extract_require_spec(l) {
                if mocked_specs.iter().any(|m| m == &spec) {
                    // `var import_X = <rhs>;` -> `import_X = <rhs>;` (re-fetch -> mock)
                    rebinds.push(l.replacen("var ", "", 1));
                }
            }
        }
    }
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
    out.join("\n")
}

/// Extract the first string literal ("..." or '...') from `s`.
fn extract_first_string_literal(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            let quote = c;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            return s.get(start..j).map(|x| x.to_string());
        }
        i += 1;
    }
    None
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
    let factories: String = mocks.iter().map(|(_, f)| f.as_str()).collect::<Vec<_>>().join("\n");
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        let (kw, is_const) = if let Some(r) = t.strip_prefix("let ") { (Some(r), false) }
            else if let Some(r) = t.strip_prefix("var ") { (Some(r), false) }
            else if let Some(r) = t.strip_prefix("const ") { (Some(r), true) } else { (None, false) };
        if let Some(rest) = kw {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
            if name.is_empty() || out.contains(&name) {
                continue;
            }
            // let/var: share when the factory ASSIGNS the name (`name = ...`, the capture pattern).
            // const: can't be reassigned, so share only an empty-collection accumulator (`= []`/
            // `{}`/`null`) the factory mutates/reads (e.g. `capturedQueryFns.push(...)`). The
            // length gate + literal-collection gate keep short/common/value names safe.
            let shareable = if is_const {
                // const can't be reassigned → only an ARRAY accumulator (`= []`) the factory
                // mutates/reads. Arrays (`.push`) are rarely used in object shorthand/keys, so
                // word-replacing them is safe; objects/values are NOT (`{ x }`, `x:` would break).
                // array accumulator only (`= []`), tolerating a TS type annotation whose `=>`
                // would fool a naive split. len>=12: only long unique names; short ones (or mock
                // fns used in shorthand/strings) risk corrupting other code.
                let is_array = rest.contains("= []") || rest.contains("= [\n") || rest.trim_end().ends_with("= [");
                name.len() >= 12 && is_array && contains_word(&factories, &name)
            } else {
                // let/var: share when the factory REFERENCES the binding (reads or writes) and
                // it's assigned somewhere — either by the factory itself (the capture pattern: the
                // factory writes, the test reads) or by the test bodies (the inverse: the factory
                // closes over a mutable `let` that beforeEach/it reassign, e.g.
                // `let hookValue; vi.mock(..., () => ({ useX: () => hookValue })); it(() => { hookValue = ... })`).
                // Without sharing, the prepass factory runs in a scope where the let isn't bound →
                // `hookValue is not defined`. assigns_word(src) covers both directions (src includes
                // the factory); contains_word(factories) keeps it to bindings the factory actually uses.
                name.len() >= 6 && contains_word(&factories, &name) && assigns_word(src, &name)
            };
            // Skip if the name is used in object-literal shorthand (`{ name }`) — word-replacing
            // it to `globalThis.__ms.name` there is invalid JS (would break the whole file).
            if shareable && !used_in_shorthand(src, &name) {
                out.push(name);
            }
        }
    }
    out
}

/// True if `word` appears as object-literal shorthand (`{ word }`, `{ word, ...}`, `..., word }`):
/// bounded by `{`/`,` before and `}`/`,` after (whitespace allowed), so it can't be safely
/// rewritten to `globalThis.__ms.word`.
fn used_in_shorthand(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut i = 0;
    while let Some(p) = text[i..].find(word) {
        let at = i + p;
        i = at + 1;
        if at > 0 && ident(b[at - 1]) {
            continue;
        }
        let end = at + word.len();
        if end < b.len() && ident(b[end]) {
            continue;
        }
        // scan back/forward over whitespace (incl newlines)
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
        // Object-literal shorthand: `{ word }`, `{ word, ...}`, `{..., word }`. (`,word,` middle
        // is ambiguous with an arg list, so left alone.) `[`/`]` are array elements — safe.
        if (left == b'{' && (right == b'}' || right == b',')) || (left == b',' && right == b'}') {
            return true;
        }
    }
    false
}

/// True if `text` assigns `word` (`word =`, not `==`/`===`/`<=`/`>=`/`!=`), as a whole word.
fn assigns_word(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut i = 0;
    while let Some(p) = text[i..].find(word) {
        let at = i + p;
        i = at + 1;
        if at > 0 && ident(b[at - 1]) {
            continue;
        }
        let mut j = at + word.len();
        if j < b.len() && ident(b[j]) {
            continue;
        }
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        // `=` that is not part of ==, =>, etc.; and the char before `=` isn't !,<,>
        if j < b.len() && b[j] == b'=' && b.get(j + 1) != Some(&b'=') && b.get(j + 1) != Some(&b'>') {
            return true;
        }
    }
    false
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
    let kws = ["beforeEach", "beforeAll", "afterEach", "afterAll", "describe", "test", "it", "fit", "xit", "fdescribe", "xdescribe"];
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < b.len() {
        // at an identifier boundary, see if a test keyword starts here
        let boundary = i == 0 || !ident(b[i - 1]);
        let mut matched = None;
        if boundary {
            for kw in kws.iter() {
                let kb = kw.as_bytes();
                if b[i..].starts_with(kb) {
                    // next char must end the identifier (so `item`/`description` don't match `it`/`describe`)
                    let after = i + kb.len();
                    if after >= b.len() || !ident(b[after]) {
                        matched = Some(after);
                        break;
                    }
                }
            }
        }
        if let Some(mut j) = matched {
            // skip whitespace + chained `.only` / `.skip` / `.each(...)` / `.concurrent` etc.
            // up to the opening '(' of the call.
            // allow the esbuild `(0, import_vitest.it)(...)` form: skip whitespace, `.`, idents,
            // and a single closing `)` before the call's opening `(`.
            let mut found_paren = false;
            while j < b.len() {
                let c = b[j];
                if c == b'(' { found_paren = true; break; }
                if c.is_ascii_whitespace() || c == b'.' || c == b')' || ident(c) { j += 1; continue; }
                break; // something else → not a call we recognize
            }
            if found_paren {
                // balance parens from j, skipping strings/templates/comments, then drop the span.
                let mut depth = 0i32;
                let mut k = j;
                while k < b.len() {
                    match b[k] {
                        b'(' => depth += 1,
                        b')' => { depth -= 1; if depth == 0 { k += 1; break; } }
                        b'\'' | b'"' | b'`' => {
                            let q = b[k]; k += 1;
                            while k < b.len() { if b[k] == b'\\' { k += 2; continue; } if b[k] == q { break; } k += 1; }
                        }
                        b'/' if k + 1 < b.len() && b[k + 1] == b'/' => { while k < b.len() && b[k] != b'\n' { k += 1; } }
                        b'/' if k + 1 < b.len() && b[k + 1] == b'*' => { k += 2; while k + 1 < b.len() && !(b[k] == b'*' && b[k + 1] == b'/') { k += 1; } k += 1; }
                        _ => {}
                    }
                    k += 1;
                }
                // emit the keyword name (so a require used in the callee position stays visible)
                out.push_str(&text[i..j]);
                i = k;
                continue;
            }
        }
        // default: copy this char (advance by full UTF-8 char to keep valid boundaries)
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
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
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        // Skip string-literal interiors: a shared-let name that also appears as a quoted string
        // (e.g. `searchParams.get(k => k === 'mockRole' ? mockRole : null)`) must NOT have the
        // STRING rewritten — only the identifier. Copy the whole literal verbatim, honoring `\`
        // escapes. (Template `${...}` expressions are skipped too — a missed interpolation would
        // surface as a visible ReferenceError, never the silent corruption of a rewritten string.)
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            out.push(c as char);
            i += 1;
            while i < text.len() {
                let d = b[i];
                if d == b'\\' && i + 1 < text.len() {
                    out.push_str(&text[i..i + 2]);
                    i += 2;
                    continue;
                }
                let ch = text[i..].chars().next().unwrap();
                out.push_str(&text[i..i + ch.len_utf8()]);
                i += ch.len_utf8();
                if d == c {
                    break;
                }
            }
            continue;
        }
        if text[i..].starts_with(word) {
            // not preceded by an identifier char, nor a property-access `.` (so `obj.word` stays
            // intact). A spread `...word` (the `.` is part of `...`) IS the variable — rewrite it.
            let prop_dot = i > 0 && b[i - 1] == b'.' && !(i >= 2 && b[i - 2] == b'.');
            // accessor name (`get word()` / `set word()`): the NAME is a property key, not a
            // variable read — rewriting it yields `get globalThis.__ms.word()` (a syntax error).
            // The getter BODY's `return word` is preceded by `return `, so it still rewrites.
            let accessor_name = {
                let mut j = i;
                while j > 0 && (b[j - 1] == b' ' || b[j - 1] == b'\t') {
                    j -= 1;
                }
                j >= 3
                    && (&b[j - 3..j] == b"get" || &b[j - 3..j] == b"set")
                    && (j == 3 || !ident(b[j - 4]))
                    && j < i // there was whitespace between the keyword and the name
            };
            let before_ok = i == 0 || (!ident(b[i - 1]) && !prop_dot && !accessor_name);
            let after = i + w.len();
            // skip object-literal keys (`word:`) — replacing them yields `globalThis.__ms.word:`.
            let after_ok = after >= b.len() || (!ident(b[after]) && b[after] != b':');
            if before_ok && after_ok {
                out.push_str(repl);
                i = after;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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
/// - replace `__toESM` (which COPIES the namespace) with one that returns the SAME exports
///   object (adding `default` for CJS interop) so `import * as ns` + `import {x}` share one
///   object — `vi.spyOn(ns,'x')` is then seen by every importer (vitest semantics).
fn postprocess_mr_cjs(mut code: String) -> String {
    code = code.replace(
        "var __defProp = Object.defineProperty;",
        "var __defProp = (o, k, d) => Object.defineProperty(o, k, Object.assign({ configurable: true }, d));",
    );
    code = code.replace(
        "var __toESM = ",
        "var __toESM = (m) => (m && m.__esModule ? m : (m && (typeof m === \"object\" || typeof m === \"function\") && !(\"default\" in m) && Object.defineProperty(m, \"default\", { value: m, configurable: true }), m)); var __toESM_unused = ",
    );
    code
}

/// Module-runner: is it enabled? (default on; TURBO_NO_MR disables → legacy bundle path)
fn mr_enabled() -> bool {
    std::env::var("TURBO_NO_MR").is_err()
}

/// react family — bundled standalone (one instance) and externalized from every other
/// node_modules bundle so all code shares the SAME react (fixes dual-react in mocks).
fn is_react_family(spec_or_path: &str) -> bool {
    let p = spec_or_path;
    p.ends_with("/react") || p == "react"
        || p.contains("/react/") || p.contains("react-dom") || p.contains("react/jsx")
        || p.ends_with("/scheduler") || p == "scheduler"
}

/// Bundle a node_modules entry to CJS (one esbuild call per package entry), with react family
/// externalized so it's shared. Returns the bundled code. Cached by entry+mtime.
fn esbuild_bundle_dep_cjs(abs: &Path) -> Option<String> {
    let root = project_root(abs)?;
    let esbuild = root.join("node_modules/.bin/esbuild");
    use std::hash::{Hash, Hasher};
    let mtime = std::fs::metadata(abs).ok().and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    abs.hash(&mut h); mtime.hash(&mut h); "mr-dep-v1".hash(&mut h);
    let cache = cache_dir().join(format!("mrdep-{:016x}.cjs", h.finish()));
    if let Ok(c) = std::fs::read_to_string(&cache) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(c);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let mut cmd = std::process::Command::new(&esbuild);
    cmd.current_dir(&root).arg(abs).args([
        "--bundle", "--format=cjs", "--platform=browser", "--jsx=automatic",
        "--loader:.css=empty", "--loader:.scss=empty", "--loader:.svg=text",
        "--loader:.png=dataurl", "--loader:.jpg=dataurl", "--loader:.gif=dataurl", "--loader:.webp=dataurl",
        "--log-level=silent", "--define:process.env.NODE_ENV=globalThis.process.env.NODE_ENV",
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
    let code = postprocess_mr_cjs(String::from_utf8(out.stdout).ok()?);
    write_atomic(&cache, &code);
    Some(code)
}

fn read_transformed(abs: &Path, as_cjs: bool, prefer_metadata: bool) -> Option<String> {
    // Module-runner: per-module CJS transform (app) / per-package CJS bundle (node_modules)
    // so imports are live + mockable and react is shared. Only on the CJS load path (the test
    // entry + its require-graph); the legacy ESM path (DOM boot, setup files) stays oxc.
    if as_cjs && mr_enabled() {
        // App code: transform per-file (live bindings → spyOn/vi.mock work). node_modules:
        // bundle-per-package (fast), but the context-bearing packages (react, @mui/*, @emotion/*)
        // are externalized from every bundle so their shared singletons (ThemeContext, emotion
        // cache, react) resolve to ONE instance via the require cache (no dual-context).
        if abs.components().any(|c| c.as_os_str() == "node_modules") {
            if let Some(code) = esbuild_bundle_dep_cjs(abs) {
                return Some(code);
            }
        } else if let Some(code) = esbuild_transform_cjs(abs, prefer_metadata) {
            return Some(code);
        }
    }
    let raw = std::fs::read_to_string(abs).ok()?;
    if !crate::transform::needs_transform(abs) {
        return Some(raw);
    }
    // content-addressed disk cache, shared across runs and workers
    let path = cache_dir().join(format!("{:016x}.js", cache_key(abs, &raw)));
    if let Ok(cached) = std::fs::read_to_string(&path) {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Some(cached);
    }
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let out = maybe_transform(abs, raw)
        .map_err(|e| eprintln!("transform {}: {e}", abs.display()))
        .ok()?;
    let _ = std::fs::write(&path, &out);
    Some(out)
}

// ---- vi.mock hoisting -----------------------------------------------------

/// Extract `vi.mock("spec", factory)` calls from source (string-literal specifier).
/// Returns (specifier, factory_source). Models vitest's hoisting: mocks are registered
/// before the module graph instantiates. Dynamic specifiers/automock: M-later.
fn extract_mocks(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    // Match both `vi.mock(` (vitest) and `jest.mock(` (jest) — same hoisting semantics.
    while let Some((rel, needle_len)) = src.get(i..).and_then(|s| {
        let a = s.find("vi.mock(").map(|p| (p, "vi.mock(".len()));
        let b = s.find("jest.mock(").map(|p| (p, "jest.mock(".len()));
        match (a, b) {
            (Some(x), Some(y)) => Some(if x.0 <= y.0 { x } else { y }),
            (Some(x), None) => Some(x),
            (None, Some(y)) => Some(y),
            (None, None) => None,
        }
    }) {
        let start = i + rel + needle_len;
        // parse first arg: a string literal
        let rest = src.get(start..).unwrap_or("");
        let q = rest.trim_start();
        let lead = rest.len() - q.len();
        let Some(quote) = q.chars().next() else { break };
        if quote != '"' && quote != '\'' && quote != '`' {
            i = start;
            continue;
        }
        let after_q = &q[1..];
        let Some(end_q) = after_q.find(quote) else { break };
        let spec = after_q[..end_q].to_string();
        // find factory: from after the spec's closing quote to the balanced ')'
        let mut j = start + lead + 1 + end_q + 1;
        // skip to comma (factory) or ')' (no factory)
        let tail = src.get(j..).unwrap_or("");
        let mut factory = String::new();
        if let Some(comma_rel) = tail.find(',') {
            let fstart = j + comma_rel + 1;
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
            // look back for the `{ ... }` clause of this import statement
            let head = text.get(..at).unwrap_or("");
            if let Some(close) = head.rfind('}') {
                if let Some(open) = head.get(..close).unwrap_or("").rfind('{') {
                    for part in head.get(open + 1..close).unwrap_or("").split(',') {
                        let name = part.trim().split_whitespace().last().unwrap_or("").trim();
                        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                            out.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

fn register_mock_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
    exports_val: v8::Local<v8::Value>,
) -> Option<()> {
    let global = scope.get_current_context().global(scope);
    let mut names = object_keys(scope, global, exports_val);
    // Isolate-reuse: if this path was ALREADY loaded (e.g. a setup mock, or a real module a
    // cached importer holds a reference to), a test file re-mocking it must NOT swap in a new
    // exports object — cached CJS importers hold the old object and read its props live, so they
    // would keep seeing the old mock (e.g. the analytics trackEvent the test spies on differs
    // from the one a cached hook calls → "0 calls"). Instead, mutate the existing object in
    // place: copy the new mock's props onto it. Identity is preserved; every importer sees the
    // update. (Fresh mode reloads every module per file, so this only matters under reuse.)
    if reuse_isolate_enabled() {
        if let Some(existing_g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(abs).cloned()) {
            let existing = v8::Local::new(scope, &existing_g);
            if existing.is_object() && exports_val.is_object() {
                if let (Some(dst), Some(src)) = (existing.to_object(scope), exports_val.to_object(scope)) {
                    for n in &names {
                        if let Some(k) = v8::String::new(scope, n) {
                            if let Some(val) = src.get(scope, k.into()) {
                                dst.set(scope, k.into(), val);
                            }
                        }
                    }
                    REGISTRY.with(|r| r.borrow_mut().mocks.insert(abs.to_path_buf(), String::new()));
                    invalidate_importers(abs);
                    return Some(());
                }
            }
        }
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
        reg.cjs_synth_by_path.insert(abs.to_path_buf(), synth_global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
        reg.mocks.insert(abs.to_path_buf(), String::new());
    });
    if reuse_isolate_enabled() {
        invalidate_importers(abs);
    }
    Some(())
}

/// Register a mock by running its factory source (self-contained `() => (...)`).
fn register_mock<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path, factory_src: &str) -> Option<()> {
    let exports_val = if factory_src.is_empty() {
        v8::Object::new(scope).into()
    } else {
        let code = v8::String::new(scope, &format!("({factory_src})"))?;
        let script = v8::Script::compile(scope, code, None)?;
        let f = v8::Local::<v8::Function>::try_from(script.run(scope)?).ok()?;
        let undef = v8::undefined(scope).into();
        f.call(scope, undef, &[])?
    };
    register_mock_value(scope, abs, exports_val)
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
        let Some(item_val) = arr.get_index(scope, i) else { continue };
        let Some(item) = v8::Local::<v8::Object>::try_from(item_val).ok() else {
            continue;
        };
        let Some(spec) = item.get(scope, spec_key.into()).map(|v| v.to_rust_string_lossy(scope)) else {
            continue;
        };
        let _ = skip_relative;
        let exports = item.get(scope, exp_key.into()).unwrap_or_else(|| v8::undefined(scope).into());
        if exports.is_promise() || is_thenable(scope, exports) {
            retained.push(item_val); // unresolved factory — defer to the outer drain
            continue;
        }
        if exports.is_undefined() {
            continue; // failed/async-rejected factory — fall back to the real module
        }
        if let Some(abs) = resolve_spec(&spec, base_dir) {
            register_mock_value(scope, &abs, exports);
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
    let Ok(obj) = v8::Local::<v8::Object>::try_from(val) else { return false };
    let Some(k) = v8::String::new(scope, "then") else { return false };
    obj.get(scope, k.into()).map(|v| v.is_function()).unwrap_or(false)
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
    "cc-v1".hash(&mut h);
    cache_dir().join(format!("cc-{:016x}.bin", h.finish()))
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) {
    let seq = ATOMIC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{}", std::process::id(), seq));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Compile the wrapped CJS source, consuming a persisted bytecode cache when present and
/// producing one on a miss. Returns the bound Script. Falls back to a plain compile on any
/// cache miss/reject (V8 recompiles internally and we keep that result).
fn compile_cjs_cached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: v8::Local<v8::String>,
    wrapped: &str,
) -> Option<v8::Local<'s, v8::Script>> {
    use v8::script_compiler as sc;
    let path = cc_path(wrapped);
    if let Ok(bytes) = std::fs::read(&path) {
        if !bytes.is_empty() {
            let cached = v8::CachedData::new(&bytes);
            let mut source = sc::Source::new_with_cached_data(code, None, cached);
            let compiled = sc::compile(scope, &mut source, sc::CompileOptions::ConsumeCodeCache, sc::NoCacheReason::NoReason);
            // `bytes` stays alive through this block (Source borrowed it for the compile).
            if let Some(s) = compiled {
                return Some(s);
            }
        }
    }
    // Miss/reject: compile fresh and persist the bytecode for next time.
    let mut source = sc::Source::new(code, None);
    let script = sc::compile(scope, &mut source, sc::CompileOptions::NoCompileOptions, sc::NoCacheReason::NoReason)?;
    if let Some(cc) = script.get_unbound_script(scope).create_code_cache() {
        write_atomic_bytes(&path, &cc);
    }
    Some(script)
}

fn load_cjs<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path, drain_mocks: bool) -> Option<()> {
    load_cjs_inner(scope, abs, drain_mocks, false)
}

fn load_cjs_inner<'s>(scope: &mut v8::PinScope<'s, '_>, abs: &Path, drain_mocks: bool, prefer_metadata: bool) -> Option<()> {
    if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(abs)) {
        return Some(());
    }
    let Some(raw) = read_transformed(abs, true, prefer_metadata) else {
        eprintln!("  cjs read/transform failed: {}", abs.display());
        return None;
    };
    let wrapped =
        format!("(function (exports, module, require, __filename, __dirname) {{\n{raw}\n}})");
    let code = v8::String::new(scope, &wrapped)?;
    // Under --coverage, give the script an origin named after the file so V8's precise-coverage
    // report carries a URL we can map back to source. Off by default (origin None) — zero change.
    let script = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let origin = if crate::coverage::enabled() {
            v8::String::new(tc, &abs.to_string_lossy()).map(|name| {
                v8::ScriptOrigin::new(tc, name.into(), 0, 0, false, 123, None, false, false, false, None)
            })
        } else {
            None
        };
        let compiled = if code_cache_enabled() && origin.is_none() {
            compile_cjs_cached(tc, code, &wrapped)
        } else {
            match &origin {
                Some(o) => v8::Script::compile(tc, code, Some(o)),
                None => v8::Script::compile(tc, code, None),
            }
        };
        match compiled {
            Some(s) => Some(v8::Global::new(tc, s)),
            None => {
                eprintln!(
                    "  cjs compile failed: {} :: {}",
                    abs.display(),
                    tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_default()
                );
                None
            }
        }
    };
    let script = v8::Local::new(scope, &script?);
    let wrapper = v8::Local::<v8::Function>::try_from(script.run(scope)?).ok()?;

    let exports_obj = v8::Object::new(scope);
    let module_obj = v8::Object::new(scope);
    let exports_key = v8::String::new(scope, "exports")?;
    module_obj.set(scope, exports_key.into(), exports_obj.into());

    let dir = abs.parent().unwrap();
    let dir_str = v8::String::new(scope, &dir.to_string_lossy())?;
    let global = scope.get_current_context().global(scope);
    let mk_key = v8::String::new(scope, "__mkRequire")?;
    let mk = v8::Local::<v8::Function>::try_from(global.get(scope, mk_key.into())?).ok()?;
    let undef = v8::undefined(scope).into();
    let esm_flag = v8::Boolean::new(scope, is_esm_module(abs));
    let filename = v8::String::new(scope, &abs.to_string_lossy())?;
    // pass this module's path as the require importer, so native_require can record the import
    // edge (imported <- importer) for mock-graph invalidation under isolate-reuse.
    let require = mk.call(scope, undef, &[dir_str.into(), esm_flag.into(), filename.into()])?;
    // expose this module's dir so runtime vi.mock() (hoisted above the requires) can resolve a
    // relative spec + register immediately, before the module's consumer requires run.
    let prev_dir = if let Some(k) = v8::String::new(scope, "__ttDir") {
        let prev = global.get(scope, k.into());
        global.set(scope, k.into(), dir_str.into());
        prev
    } else {
        None
    };
    // mark in-progress (with its module object) so a circular require() returns live exports
    let module_global = v8::Global::new(scope, module_obj);
    REGISTRY.with(|r| r.borrow_mut().loading.insert(abs.to_path_buf(), module_global));
    let call_res = {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        let r = wrapper.call(
            tc,
            undef,
            &[exports_obj.into(), module_obj.into(), require, filename.into(), dir_str.into()],
        );
        if r.is_none() && tc.has_caught() && std::env::var("TURBO_CJS_DEBUG").is_ok() {
            let stk = tc.stack_trace().map(|s| s.to_rust_string_lossy(tc)).unwrap_or_default();
            eprintln!(
                "  cjs body threw: {} :: {} :: {}",
                abs.display(),
                tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_default(),
                stk.lines().take(4).collect::<Vec<_>>().join(" | ")
            );
        }
        r
    };
    REGISTRY.with(|r| r.borrow_mut().loading.remove(abs));
    if let (Some(k), Some(prev)) = (v8::String::new(scope, "__ttDir"), prev_dir) {
        global.set(scope, k.into(), prev);
    }
    if call_res.is_none() {
        // The module body threw. If it's a decorator file in an emitDecoratorMetadata project and
        // we haven't already retried, the throw is often a missing-`design:type` error (Mongoose
        // @Prop / Sequelize @Column read metadata at class-definition). Re-transform THIS file via
        // the metadata path (ts.transpileModule) and re-run once. Scoped to this file only — its
        // require()d deps still load on the default esbuild path, so files that load fine without
        // metadata never change (no regression).
        if !prefer_metadata
            && !abs.components().any(|c| c.as_os_str() == "node_modules")
            && decorator_metadata_enabled(abs)
            && std::fs::read_to_string(abs).map(|s| file_has_decorator(&s)).unwrap_or(false)
        {
            return load_cjs_inner(scope, abs, drain_mocks, true);
        }
        return None;
    }

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

    let final_exports = module_obj.get(scope, exports_key.into())?;
    let names = object_keys(scope, global, final_exports);
    let exports_global = v8::Global::new(scope, final_exports);
    let synth = make_synthetic(scope, &names)?;
    let hash = synth.get_identity_hash().get();
    let synth_global = v8::Global::new(scope, synth);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.real_exports.insert(abs.to_path_buf(), exports_global.clone());
        reg.cjs_exports.insert(abs.to_path_buf(), exports_global);
        reg.cjs_export_names.insert(abs.to_path_buf(), names);
        reg.cjs_synth_by_path.insert(abs.to_path_buf(), synth_global);
        reg.path_by_hash.insert(hash, abs.to_path_buf());
    });
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
        .filter_map(|i| arr.get_index(scope, i).map(|v| v.to_rust_string_lossy(scope)))
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
    Some(v8::Module::create_synthetic_module(scope, mod_name, &export_names, synth_eval))
}

fn synth_eval<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let hash = module.get_identity_hash().get();
    let (exports_g, names) = REGISTRY.with(|r| {
        let reg = r.borrow();
        let path = reg.path_by_hash.get(&hash)?.clone();
        Some((
            reg.cjs_exports.get(&path)?.clone(),
            reg.cjs_export_names.get(&path).cloned().unwrap_or_default(),
        ))
    })?;
    let exports = v8::Local::new(scope, &exports_g);
    let dkey = v8::String::new(scope, "default")?;
    module.set_synthetic_module_export(scope, dkey, exports)?;
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
      if (typeof k === 'symbol') return undefined;
      if (k === '__turboLazyStub') return true;
      if (k === 'default') return cache['__default'] || (cache['__default'] = make(DEFAULT_NAME || 'default'));
      return cache[k] || (cache[k] = make(k));
    }},
    set: function(t, k, v) {{ cache[k] = v; return true; }},
    has: function() {{ return true; }},
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
    REGISTRY.with(|r| r.borrow_mut().lazy_stub_ns.insert(cache_key.to_string(), g.clone()));
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
        if a.is_undefined() { true } else { a.boolean_value(scope) }
    };
    // node builtins (fs/path/module/...) -> shimmed module objects
    if is_node_builtin(&spec) {
        ensure_node_builtin(scope, &spec);
        let key = PathBuf::from(format!("<node:{spec}>"));
        if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&key).cloned()) {
            rv.set(v8::Local::new(scope, &g));
        }
        return;
    }
    // vitest builtin (describe/it/expect/vi from globals) — never load the real package.
    if spec == "vitest" {
        ensure_vitest_builtin(scope);
        if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&vitest_key()).cloned()) {
            rv.set(v8::Local::new(scope, &g));
        }
        return;
    }
    // Layer B: configured barrel (TURBO_LAZY_STUBS) -> lazy Proxy namespace, never resolved or
    // evaluated. Falls through to the real load if the stub factory fails.
    if let Some((key, def)) = lazy_stub_match(&spec) {
        if let Some(ns) = ensure_lazy_stub(scope, &key, def.as_deref()) {
            rv.set(ns);
            return;
        }
    }
    let Some(abs) = resolve_spec_as(&spec, Path::new(&dir), esm) else {
        // A node_modules module requiring something unresolvable in this env (a sibling
        // package.json via "..", an optional dep) — return a stub so it doesn't crash. Keyed by a
        // synthetic path so repeats hit the cache.
        if stub_failed_deps() && dir.contains("/node_modules/") {
            let key = PathBuf::from(format!("<unresolved:{dir}:{spec}>"));
            if REGISTRY.with(|r| !r.borrow().cjs_exports.contains_key(&key)) {
                let _ = register_node_stub(scope, &key);
            }
            if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&key).cloned()) {
                rv.set(v8::Local::new(scope, &g));
                return;
            }
        }
        throw(scope, &format!("cannot resolve {spec}"));
        return;
    };
    // Record the import edge (imported <- importer) for mock-graph invalidation under reuse.
    if reuse_isolate_enabled() {
        let importer = args.get(3);
        if !importer.is_undefined() {
            let imp = PathBuf::from(importer.to_rust_string_lossy(scope));
            REGISTRY.with(|r| {
                r.borrow_mut().import_edges.entry(abs.clone()).or_default().insert(imp);
            });
        }
    }
    // circular dependency: module abs is mid-execution → return its live (partial) exports.
    if let Some(modg) = REGISTRY.with(|r| r.borrow().loading.get(&abs).cloned()) {
        let mod_obj = v8::Local::new(scope, &modg);
        if let Some(ek) = v8::String::new(scope, "exports") {
            if let Some(exp) = mod_obj.get(scope, ek.into()) {
                rv.set(exp);
            }
        }
        return;
    }
    // CSS/asset/JSON imports -> stub via load_dep (CSS-module proxy / parsed JSON value)
    if is_asset(&abs) || abs.extension().and_then(|e| e.to_str()) == Some("json") {
        load_dep(scope, &abs);
        if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned()) {
            rv.set(v8::Local::new(scope, &g));
        }
        return;
    }
    // native addon (.node) -> load via the Node-API host
    if abs.extension().and_then(|e| e.to_str()) == Some("node") {
        let cached = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned());
        let g = match cached {
            Some(g) => g,
            None => match crate::napi_host::load_addon(scope, &abs) {
                Ok(g) => {
                    REGISTRY.with(|r| r.borrow_mut().cjs_exports.insert(abs.clone(), g.clone()));
                    g
                }
                Err(e) => {
                    // A native addon that won't dlopen in this env (e.g. fsevents on a CI box, or
                    // an arch mismatch) — stub it so an optional require() (chokidar's fsevents)
                    // doesn't take the file down.
                    if stub_failed_deps() && register_node_stub(scope, &abs).is_some() {
                        if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned()) {
                            rv.set(v8::Local::new(scope, &g));
                        }
                        return;
                    }
                    throw(scope, &format!("native addon {}: {e}", abs.display()));
                    return;
                }
            },
        };
        rv.set(v8::Local::new(scope, &g));
        return;
    }
    // mocked?
    let mocked = REGISTRY.with(|r| r.borrow().mocks.get(&abs).cloned());
    if let Some(factory) = mocked {
        if !REGISTRY.with(|r| r.borrow().cjs_exports.contains_key(&abs)) {
            register_mock(scope, &abs, &factory);
        }
    } else if mr_enabled() || detect_kind(&abs) == Kind::Cjs {
        // MR: load everything as CJS (esbuild --format=cjs) so imports are live + mockable.
        if load_cjs(scope, &abs, true).is_none() {
            // A node_modules dep that can't load in the shimmed env -> stub it (don't crash the
            // importer). App modules still hard-fail (a real bug there must surface).
            if abs.to_string_lossy().contains("/node_modules/") && stub_failed_deps()
                && register_node_stub(scope, &abs).is_some()
            {
                // fall through to return the stub
            } else {
                throw(scope, &format!("failed to load {spec}"));
                return;
            }
        }
    } else {
        // require() of ESM (Node 22+ behavior): load + instantiate + evaluate, return the
        // module namespace as the require() result. Handles CJS->ESM requires (e.g. tslib).
        let Some(module) = load_graph(scope, &abs) else {
            if abs.to_string_lossy().contains("/node_modules/") && stub_failed_deps()
                && register_node_stub(scope, &abs).is_some()
            {
                if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned()) {
                    rv.set(v8::Local::new(scope, &g));
                }
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
        return;
    }
    if let Some(g) = REGISTRY.with(|r| r.borrow().cjs_exports.get(&abs).cloned()) {
        let local = v8::Local::new(scope, &g);
        rv.set(local);
    }
}

/// __ttRegisterMock(dir, spec, exports): register a mock immediately (vi.mock hoisting). The
/// factory ran in the calling module's scope, so `exports` already closes over the right vars.
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
    if let Some(abs) = resolve_spec(&spec, Path::new(&dir)) {
        register_mock_value(scope, &abs, exports);
    }
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
    let Some(abs) = resolve_spec(&spec, Path::new(&dir)) else { return };
    // importActual/importOriginal returns the REAL module, NOT a registered mock (else a factory
    // doing `mockImplementation(actual.fn)` points the spy at itself → infinite recursion).
    // Prefer the genuine exports cached when the real module was loaded (no re-run → no dual
    // instances of singletons like react-query). If never really loaded, load it once with the
    // mock records temporarily removed, then restore them.
    if let Some(g) = REGISTRY.with(|r| r.borrow().real_exports.get(&abs).cloned()) {
        rv.set(v8::Local::new(scope, &g));
        return;
    }
    let saved = REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        if reg.mocks.contains_key(&abs) {
            let m = reg.mocks.remove(&abs);
            let e = reg.cjs_exports.remove(&abs);
            let s = reg.cjs_synth_by_path.remove(&abs);
            let n = reg.cjs_export_names.remove(&abs);
            Some((m, e, s, n))
        } else {
            None
        }
    });
    let _ = load_cjs(scope, &abs, true);
    let real = REGISTRY.with(|r| r.borrow().real_exports.get(&abs).cloned()
        .or_else(|| r.borrow().cjs_exports.get(&abs).cloned()));
    if let Some((m, e, s, n)) = saved {
        REGISTRY.with(|r| {
            let mut reg = r.borrow_mut();
            if let Some(m) = m { reg.mocks.insert(abs.clone(), m); }
            if let Some(e) = e { reg.cjs_exports.insert(abs.clone(), e); }
            if let Some(s) = s { reg.cjs_synth_by_path.insert(abs.clone(), s); }
            if let Some(n) = n { reg.cjs_export_names.insert(abs.clone(), n); }
        });
    }
    if let Some(g) = real {
        rv.set(v8::Local::new(scope, &g));
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
    if let Some(abs) = resolve_spec(&spec, Path::new(&dir)) {
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

fn throw(scope: &mut v8::PinScope, msg: &str) {
    let m = v8::String::new(scope, msg).unwrap();
    let exc = v8::Exception::error(scope, m);
    scope.throw_exception(exc);
}

// ---- ESM graph ------------------------------------------------------------

fn load_graph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    abs: &Path,
) -> Option<v8::Local<'s, v8::Module>> {
    if let Some(g) = REGISTRY.with(|r| r.borrow().esm_by_path.get(abs).cloned()) {
        return Some(v8::Local::new(scope, &g));
    }
    let raw = read_transformed(abs, false, false)?;
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
    // A generated bundle in the cache dir has no node_modules beside it; resolve its externalized
    // bare imports relative to the recorded source dir (set by esbuild_bundle_full).
    let dir = REGISTRY
        .with(|r| r.borrow().bundle_src_dir.get(abs).cloned())
        .or_else(|| abs.parent().map(|p| p.to_path_buf()))
        .unwrap();
    for i in 0..requests.length() {
        let req = v8::Local::<v8::ModuleRequest>::try_from(requests.get(scope, i).unwrap()).unwrap();
        let spec = req.get_specifier().to_rust_string_lossy(scope);
        if spec == "vitest" {
            ensure_vitest_builtin(scope)?;
            continue;
        }
        if is_node_builtin(&spec) {
            ensure_node_builtin(scope, &spec)?;
            continue;
        }
        let Some(dep) = resolve_spec(&spec, &dir) else {
            eprintln!("cannot resolve {spec} from {}", dir.display());
            return None;
        };
        load_dep(scope, &dep)?;
    }
    Some(module)
}

/// Registry key for the built-in `vitest` module shim (named import surface).
fn vitest_key() -> PathBuf {
    PathBuf::from("<builtin:vitest>")
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
    for n in [
        "describe", "it", "test", "expect", "vi", "beforeEach", "afterEach", "beforeAll",
        "afterAll", "assert",
    ] {
        let k = v8::String::new(scope, n)?;
        if let Some(v) = global.get(scope, k.into()) {
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
    let mocked = REGISTRY.with(|r| r.borrow().mocks.get(dep).cloned());
    if let Some(factory) = mocked {
        if !REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(dep)) {
            register_mock(scope, dep, &factory)?;
        }
        return Some(());
    }
    // CSS/asset imports -> stub (CSS-module proxy: any class name -> its own string)
    if is_asset(dep) {
        if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(dep)) {
            return Some(());
        }
        return register_mock(
            scope,
            dep,
            "() => new Proxy({}, { get: (t, p) => typeof p === 'string' ? p : undefined })",
        );
    }
    // JSON imports -> module with default = parsed value + named keys
    if dep.extension().and_then(|e| e.to_str()) == Some("json") {
        if REGISTRY.with(|r| r.borrow().cjs_synth_by_path.contains_key(dep)) {
            return Some(());
        }
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
    let lookup = if spec == "vitest" {
        ensure_vitest_builtin(scope);
        Some(vitest_key())
    } else if is_node_builtin(&spec) {
        ensure_node_builtin(scope, &spec);
        Some(PathBuf::from(format!("<node:{spec}>")))
    } else {
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
            resolve_spec(&spec, &base)
        })
    };
    let abs = lookup?;
    // Already loaded?
    if let Some(g) = REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.esm_by_path
            .get(&abs)
            .or_else(|| reg.cjs_synth_by_path.get(&abs))
            .cloned()
    }) {
        return Some(v8::Local::new(scope, &g));
    }
    // Externalized package not loaded yet (a setup bundle's `import 'react'` etc. under
    // --packages=external): lazy-load it now via the shared loaders so the bundle binds to the
    // ONE instance every other importer gets, instead of a copy baked into the bundle.
    if mr_enabled() && esbuild_transform_cjs(&abs, false).is_some() {
        load_cjs(scope, &abs, true);
    } else {
        load_dep(scope, &abs);
    }
    REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.esm_by_path
            .get(&abs)
            .or_else(|| reg.cjs_synth_by_path.get(&abs))
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
    let url = path
        .map(|p| format!("file://{}", p.display()))
        .unwrap_or_else(|| "file:///<unknown>".to_string());
    let k = v8::String::new(scope, "url").unwrap();
    let v = v8::String::new(scope, &url).unwrap();
    meta.create_data_property(scope, k.into(), v.into());
}

fn dynamic_import_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    _import_attributes: v8::Local<'s, v8::FixedArray>,
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
    // dynamic import of builtins (e.g. turbo-dom's `await import('node:module')`) + vitest
    let (abs, is_builtin) = if spec == "vitest" {
        ensure_vitest_builtin(scope);
        (Some(vitest_key()), true)
    } else if is_node_builtin(&spec) {
        ensure_node_builtin(scope, &spec);
        (Some(PathBuf::from(format!("<node:{spec}>"))), true)
    } else {
        (from_dir.as_deref().and_then(|d| resolve_spec(&spec, d)), false)
    };
    let Some(abs) = abs else {
        let err = v8::String::new(scope, &format!("dynamic import cannot resolve {spec}"))?;
        let exc = v8::Exception::error(scope, err);
        resolver.reject(scope, exc);
        return Some(promise);
    };

    // load + instantiate + evaluate the target, then resolve with its namespace
    let ns = (|| {
        if !is_builtin {
            // Module-runner: load app modules via the CJS transform path (live bindings, mocks,
            // re-runs after vi.resetModules). Falls back to the ESM graph loader otherwise.
            if mr_enabled() && esbuild_transform_cjs(&abs, false).is_some() {
                load_cjs(scope, &abs, true)?;
            } else {
                load_dep(scope, &abs)?;
            }
        }
        let module = REGISTRY.with(|r| {
            let reg = r.borrow();
            reg.esm_by_path
                .get(&abs)
                .or_else(|| reg.cjs_synth_by_path.get(&abs))
                .cloned()
        })?;
        let module = v8::Local::new(scope, &module);
        if module.get_status() == v8::ModuleStatus::Uninstantiated {
            module.instantiate_module(scope, resolve_callback)?;
        }
        if module.get_status() == v8::ModuleStatus::Instantiated {
            module.evaluate(scope)?;
        }
        Some(module.get_module_namespace())
    })();

    match ns {
        Some(ns) => {
            resolver.resolve(scope, ns);
        }
        None => {
            let err = v8::String::new(scope, &format!("dynamic import failed: {spec}"))?;
            let exc = v8::Exception::error(scope, err);
            resolver.reject(scope, exc);
        }
    }
    Some(promise)
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
fn framework_snapshot() -> &'static Vec<u8> {
    static SNAP: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    SNAP.get_or_init(|| {
        let mut creator = v8::Isolate::snapshot_creator(None, None);
        {
            v8::scope!(let scope, &mut creator);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            // Bake the REAL host platform/arch (compile-time target) into the snapshot so
            // turbo-dom's index.js loader requires the matching prebuilt .node. Without this
            // the runtime reported 'darwin'/'arm64' everywhere and non-mac hosts loaded the
            // macOS .node -> dlopen fails -> "document is not defined".
            let node_platform = match std::env::consts::OS {
                "macos" => "darwin",
                "windows" => "win32",
                other => other, // "linux", "freebsd", ...
            };
            let node_arch = match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                other => other,
            };
            let os_type = match std::env::consts::OS {
                "macos" => "Darwin",
                "windows" => "Windows_NT",
                "linux" => "Linux",
                other => other,
            };
            // isMusl() reads glibcVersionRuntime: truthy => gnu, falsy => musl.
            let glibc_runtime: &str = if cfg!(target_env = "musl") { "" } else { "2.39" };
            let prelude = format!(
                "globalThis.__ttPlatform={node_platform:?};globalThis.__ttArch={node_arch:?};globalThis.__ttOsType={os_type:?};globalThis.__ttGlibc={glibc_runtime:?};"
            );
            let pcode = v8::String::new(scope, &prelude).unwrap();
            v8::Script::compile(scope, pcode, None).unwrap().run(scope).unwrap();
            let code = v8::String::new(scope, RUNTIME_JS).unwrap();
            let s = v8::Script::compile(scope, code, None).unwrap();
            s.run(scope).unwrap();
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
        creator
            .create_blob(handling)
            .expect("framework snapshot")
            .to_vec()
    })
}

/// Re-bind native callbacks into a fresh (snapshot-derived) context.
fn install_natives(scope: &mut v8::PinScope, global: v8::Local<v8::Object>) {
    let log = |scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue| {
        let mut parts = Vec::new();
        for i in 0..args.length() {
            parts.push(args.get(i).to_rust_string_lossy(scope));
        }
        println!("[js] {}", parts.join(" "));
    };
    bind_fn(scope, global, "log", log);
    bind_fn(scope, global, "__nativeRequire", native_require);
    bind_fn(scope, global, "__ttRegisterMock", native_register_mock);
    bind_fn(scope, global, "__ttImportActual", native_import_actual);
    bind_fn(scope, global, "__ttResetModules", native_reset_modules);
    bind_fn(scope, global, "__ttUnmock", native_unmock);
    bind_fn(scope, global, "__fs_existsSync", fs_exists_sync);
    bind_fn(scope, global, "__fs_readFileSync", fs_read_file_sync);
    bind_fn(scope, global, "__fs_writeFileSync", fs_write_file_sync);
    bind_fn(scope, global, "__fs_mkdirSync", fs_mkdir_sync);
    bind_fn(scope, global, "__fs_rmSync", fs_rm_sync);
    bind_fn(scope, global, "__fs_readdirSync", fs_readdir_sync);
    if std::env::var("TURBO_STACK").is_ok() {
        if let Some(code) = v8::String::new(scope, "globalThis.__TT_STACK=true") {
            if let Some(s) = v8::Script::compile(scope, code, None) {
                s.run(scope);
            }
        }
    }
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

/// native fs.readFileSync(path, enc) -> string (utf8) — enc ignored, always returns text
fn fs_read_file_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let p = args.get(0).to_rust_string_lossy(scope);
    match std::fs::read_to_string(&p) {
        Ok(s) => {
            if let Some(v) = v8::String::new(scope, &s) {
                rv.set(v.into());
            }
        }
        Err(e) => throw(scope, &format!("readFileSync {p}: {e}")),
    }
}

/// native fs.writeFileSync(path, data)
fn fs_write_file_sync(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let data = args.get(1).to_rust_string_lossy(scope);
    if let Some(parent) = Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&p, data) {
        throw(scope, &format!("writeFileSync {p}: {e}"));
    }
}

/// native fs.mkdirSync(path, opts) — always recursive (safe)
fn fs_mkdir_sync(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let _ = std::fs::create_dir_all(&p);
}

/// native fs.rmSync / rmdirSync / unlinkSync(path) — best-effort remove
fn fs_rm_sync(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let path = Path::new(&p);
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// native fs.readdirSync(path) -> string[] of entry names
fn fs_readdir_sync(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let p = args.get(0).to_rust_string_lossy(scope);
    let arr = v8::Array::new(scope, 0);
    if let Ok(entries) = std::fs::read_dir(&p) {
        let mut i = 0;
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(s) = v8::String::new(scope, name) {
                    arr.set_index(scope, i, s.into());
                    i += 1;
                }
            }
        }
    }
    rv.set(arr.into());
}

/// node builtin module names we shim.
fn is_node_builtin(spec: &str) -> bool {
    let s = spec.strip_prefix("node:").unwrap_or(spec);
    matches!(
        s,
        "fs" | "fs/promises" | "path" | "module" | "os" | "child_process" | "url" | "util"
            | "events" | "stream" | "assert" | "assert/strict" | "perf_hooks" | "crypto"
            | "buffer" | "querystring" | "string_decoder" | "timers" | "timers/promises"
            | "http" | "http2" | "https" | "net" | "tls" | "zlib" | "dns" | "dns/promises"
            | "tty" | "constants" | "dgram" | "readline" | "v8" | "diagnostics_channel"
            | "process" | "async_hooks" | "worker_threads" | "vm" | "inspector"
    )
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
    let exports_val = builtins.get(scope, nk.into())?;
    let exports_val = if exports_val.is_undefined() {
        v8::Object::new(scope).into()
    } else {
        exports_val
    };
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
    /// Per-file environment setup time (isolate+context-from-snapshot+natives), µs.
    pub setup_us: f64,
}

impl TestReport {
    pub fn ok(&self) -> bool {
        self.failed == 0 && self.passed > 0
    }
}

/// Parse a `Profiler.takePreciseCoverage` result and attribute V8's per-range hit counts to
/// original source lines. Only project source files are reported — node_modules and test/spec
/// files are skipped (test files get hoist-reordered so their maps are unreliable, and they don't
/// belong in a coverage report anyway).
fn coverage_accumulate(json: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return };
    let Some(scripts) =
        v.get("result").and_then(|r| r.get("result")).and_then(|x| x.as_array())
    else {
        return;
    };
    for s in scripts {
        let Some(url) = s.get("url").and_then(|u| u.as_str()) else { continue };
        if url.is_empty() {
            continue;
        }
        let abs = Path::new(url);
        if !abs.is_absolute() || !abs.exists() {
            continue;
        }
        if abs.components().any(|c| c.as_os_str() == "node_modules") {
            continue;
        }
        let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains(".test.") || name.contains(".spec.") {
            continue;
        }
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
        let mut ranges: Vec<(usize, usize, i64)> = Vec::new();
        // (name, outer-range start offset, call count) — the FIRST range of each function is its
        // body span; its count is the invocation count → function coverage.
        let mut funcs: Vec<(String, usize, i64)> = Vec::new();
        if let Some(fns) = s.get("functions").and_then(|f| f.as_array()) {
            for f in fns {
                let name = f.get("functionName").and_then(|n| n.as_str()).unwrap_or("").to_string();
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
        if ranges.is_empty() {
            continue;
        }
        crate::coverage::map_script(abs, &ranges, &funcs);
        crate::coverage::map_branches(abs, &ranges);
        crate::coverage::map_statements(abs, &ranges);
    }
}

/// Run a single test file end-to-end and return its pass/fail report.
pub fn run_test_file(entry: &Path) -> Result<TestReport, String> {
    let entry_abs = std::fs::canonicalize(entry).map_err(|e| format!("{e}"))?;
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
    let mut externals: Vec<String> = Vec::new();
    let mut mock_targets: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
    for sf in &setup_files {
        let sf_dir = sf.parent().unwrap_or(Path::new("."));
        let sf_src = std::fs::read_to_string(sf).unwrap_or_default();
        for (s, factory) in extract_mocks(&sf_src) {
            // skip glob/query mock specifiers (e.g. '*?url') — not real modules.
            if s.contains('*') || s.contains('?') {
                continue;
            }
            // Skip: (a) partial mocks (spread the real module via importOriginal/importActual),
            // and (b) factories that build React elements — those must use the TEST's react
            // instance, but a prepass/setup bundle has its own react (dual-react render fail).
            // For both, leave un-externalized so the real module bundles with the test's react.
            let is_partial = factory.contains("importOriginal") || factory.contains("importActual");
            let builds_react = factory.contains("react/jsx")
                || factory.contains("createElement")
                || factory.contains("React.")
                || factory.contains("=> (")
                || factory.contains("=> <");
            if is_partial || builds_react {
                continue;
            }
            if let Some(abs) = resolve_spec(&s, sf_dir) {
                mock_targets.entry(spec_basename(&s)).or_insert(abs);
            }
            if !externals.contains(&s) {
                externals.push(s);
            }
        }
    }

    // (Entry-file vi.mock is NOT externalized: a React-component mock factory must run in the
    // TEST's react instance — a separate prepass/setup bundle has its own react (dual-react
    // render failure) — and a failed externalized mock dangles the whole file. The real module
    // bundles with the test's single react instead. Fully fixing entry component-mocks needs a
    // vitest-style per-module runner. Only setup mocks are externalized.)

    // Bundle deps with esbuild (Vite-style) so the whole dep tree loads as one clean ESM;
    // fall back to the native per-module loader if esbuild is unavailable or fails.
    // Module-runner mode skips the bundle entirely: the entry + its require-graph load
    // per-module as CJS (live bindings, shared react, in-realm mocks).
    let load_target = if mr_enabled() {
        entry_abs.clone()
    } else {
        esbuild_bundle_full(&entry_abs, &externals, &mock_targets, None).unwrap_or_else(|| entry_abs.clone())
    };

    // Make mock synthetics lenient: collect the names the bundle imports from each mock so
    // the synthetic exposes them (missing-from-factory ones export undefined, like vitest).
    if !mr_enabled() {
        if let Ok(btext) = std::fs::read_to_string(&load_target) {
            for abs in mock_targets.values() {
                let names = collect_named_imports(&btext, &abs.to_string_lossy());
                if !names.is_empty() {
                    REGISTRY.with(|r| r.borrow_mut().extra_exports.entry(abs.clone()).or_default().extend(names));
                }
            }
        }
    }

    // Zero-config speed path: reuse this worker's isolate+context across files (vitest
    // isolate:false semantics) so node_modules barrels evaluate once, not per file. Enabled by
    // the project's vitest config (`isolate: false`) or TURBO_REUSE_ISOLATE; off with TURBO_NO_REUSE.
    // FORCE_FRESH (the fresh-retry) overrides reuse for this one file. Coverage always runs the
    // fresh path (one isolate per file) — that's where the inspector collector is wired, and it
    // keeps the byte-offset → source mapping unambiguous.
    if !FORCE_FRESH.with(|f| f.get()) && reuse_decision(&entry_abs) && !crate::coverage::enabled() {
        return run_test_file_reused(&entry_abs, &setup_files, &externals, &mock_targets, &load_target);
    }

    // All Global handles live in the thread-local REGISTRY; they MUST be dropped while
    // this isolate is still alive. So every exit path runs through clear_registry()
    // before the isolate is torn down — hence the labeled block instead of `?`.
    let outcome: Result<TestReport, String> = {
        // Per-file isolation via snapshot: a fresh isolate booted from the framework
        // snapshot; Context::new yields the baked default context (framework already
        // present). We time setup (isolate+context+natives) — the M3/M0 guardrail.
        let setup_start = std::time::Instant::now();
        let blob = framework_snapshot();
        let params =
            v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(blob.clone()));
        let isolate = &mut v8::Isolate::new(params);
        isolate.set_host_initialize_import_meta_object_callback(import_meta_callback);
        isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
        // Coverage: attach an inspector to this isolate BEFORE entering scopes (needs &mut isolate).
        let mut collector =
            crate::coverage::enabled().then(|| crate::coverage::Collector::new(isolate));

        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        install_natives(scope, global);
        // Begin precise coverage now — captures module-load + test-execution counts.
        if let Some(c) = collector.as_mut() {
            c.start(context);
        }
        // process.cwd(): the test's project root (nearest package.json outside node_modules) —
        // some tests build absolute paths via `path.join(process.cwd(), 'node_modules/...')`.
        {
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
        let setup_us = setup_start.elapsed().as_secs_f64() * 1_000_000.0;

        let result = 'work: {
            // 0. DOM environment (turbo-dom) — only for files that need it, so pure-logic
            //    tests keep their clean globals (mirrors vitest per-file environment).
            if needs_dom(&entry_abs) {
                setup_dom(scope, &entry_abs);
            }

            // 0b. project setupFiles: register jest-dom matchers (expect.extend) AND any
            //     global vi.mock()s, draining each into the path-keyed mock registry.
            for sf in &setup_files {
                run_setup_file(scope, sf, &externals, &mock_targets);
            }

            // 1. entry-file vi.mock(): hoisted + registered before instantiation, via a
            //    transformed pre-pass (handles JSX/factory transforms), drained to the
            //    path-keyed mock registry. Its specifiers were externalized from the bundle.
            run_entry_mocks(scope, &entry_abs);

            // Reset the vi.hoisted index (keep the cache the prepass filled) so the entry's
            // `const x = vi.hoisted(...)` calls reuse the SAME objects the mock factories closed
            // over, by re-hitting the cache in source order.
            {
                let g = scope.get_current_context().global(scope);
                if let Some(k) = v8::String::new(scope, "__hoistedIdx") {
                    let zero = v8::Integer::new(scope, 0);
                    g.set(scope, k.into(), zero.into());
                }
            }

            // 2. load + run the test module.
            // Module-runner: load the entry as CJS (esbuild --format=cjs transform); its body
            // (describe/it + inline vi.mock) runs in-realm via the CJS wrapper, with the
            // require-graph loaded per-module (live bindings, shared react). No ESM bundle.
            // Only when the entry can actually be CJS-transformed (esbuild project present);
            // standalone ESM fixtures (no esbuild) fall through to the legacy ESM graph loader.
            if mr_enabled() && esbuild_transform_cjs(&entry_abs, false).is_some() {
                let err = {
                    let tc = std::pin::pin!(v8::TryCatch::new(scope));
                    let tc = &mut tc.init();
                    if load_cjs(tc, &entry_abs, false).is_none() {
                        Some(if tc.has_caught() {
                            tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_else(|| "entry load failed".into())
                        } else {
                            "entry load failed".to_string()
                        })
                    } else {
                        tc.perform_microtask_checkpoint();
                        None
                    }
                };
                if let Some(e) = err {
                    break 'work Err(e);
                }
                // Persist __ttDir = the entry file's dir for the test-run phase (loads cleared it).
                // So vi.doMock/vi.unmock/vi.importActual fired from inside a test callback resolve
                // relative + `@/`-aliased specs against the test file, like at hoist time.
                if let Some(d) = entry_abs.parent() {
                    let js = format!("globalThis.__ttDir = {:?};", d.to_string_lossy());
                    if let Some(code) = v8::String::new(scope, &js) {
                        if let Some(s) = v8::Script::compile(scope, code, None) { s.run(scope); }
                    }
                }
                break 'work drive_tests(scope, global);
            }
            let Some(module) = load_graph(scope, &load_target) else {
                break 'work Err("graph load failed".to_string());
            };
            {
                let tc = std::pin::pin!(v8::TryCatch::new(scope));
                let tc = &mut tc.init();
                let ok = module.instantiate_module(tc, resolve_callback);
                if ok != Some(true) {
                    let msg = if tc.has_caught() {
                        tc.exception()
                            .map(|e| e.to_rust_string_lossy(tc))
                            .unwrap_or_else(|| "unknown".into())
                    } else {
                        "instantiate failed".to_string()
                    };
                    break 'work Err(format!("instantiate: {msg}"));
                }
            }
            let ev_err: Option<String> = {
                let tc = std::pin::pin!(v8::TryCatch::new(scope));
                let tc = &mut tc.init();
                let ev = module.evaluate(tc);
                tc.perform_microtask_checkpoint();
                if tc.has_caught() {
                    Some(format!(
                        "evaluate threw: {}",
                        tc.exception()
                            .map(|e| e.to_rust_string_lossy(tc))
                            .unwrap_or_else(|| "unknown".into())
                    ))
                } else if ev.is_none() || module.get_status() != v8::ModuleStatus::Evaluated {
                    if module.get_status() == v8::ModuleStatus::Errored {
                        Some(format!(
                            "module errored: {}",
                            module.get_exception().to_rust_string_lossy(tc)
                        ))
                    } else {
                        Some(format!("evaluate failed: status={:?}", module.get_status()))
                    }
                } else {
                    None
                }
            };
            if let Some(e) = ev_err {
                break 'work Err(e);
            }

            // 3. drive the collected tests: __tt.run() -> Promise<summary>
            drive_tests(scope, global)
        };

        // Coverage: read out V8's precise counts for everything this file executed, then map the
        // byte ranges back to original source lines. Done before teardown (isolate still alive).
        if let Some(c) = collector.as_mut() {
            if let Some(json) = c.take() {
                coverage_accumulate(&json);
            }
            c.stop(context);
        }
        collector = None; // drop the inspector while the isolate + scope are still alive

        // drop all Globals while the isolate is still alive
        clear_registry();
        result.map(|mut r| {
            r.setup_us = setup_us;
            r
        })
    };
    outcome
}

/// Reuse path: run one file on this worker's persistent isolate+context. node_modules modules
/// stay evaluated across files; per-file framework + app state is reset between files.
fn run_test_file_reused(
    entry_abs: &Path,
    setup_files: &[PathBuf],
    externals: &[String],
    mock_targets: &std::collections::HashMap<String, PathBuf>,
    load_target: &Path,
) -> Result<TestReport, String> {
    let setup_start = std::time::Instant::now();
    // init the persistent isolate once per worker
    REUSE_ISO.with(|iso_cell| {
        if iso_cell.borrow().is_none() {
            let blob = framework_snapshot();
            let params =
                v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(blob.clone()));
            let mut isolate = v8::Isolate::new(params);
            isolate.set_host_initialize_import_meta_object_callback(import_meta_callback);
            isolate.set_host_import_module_dynamically_callback(dynamic_import_callback);
            *iso_cell.borrow_mut() = Some(isolate);
        }
    });

    REUSE_ISO.with(|iso_cell| {
        let mut iso_b = iso_cell.borrow_mut();
        let isolate = iso_b.as_mut().unwrap();
        v8::scope!(let scope, isolate);

        // get-or-create the persistent context (framework snapshot already baked in)
        let (context, first_file) = REUSE_CTX.with(|c| {
            let mut cb = c.borrow_mut();
            if let Some(g) = cb.as_ref() {
                (v8::Local::new(scope, g), false)
            } else {
                let ctx = v8::Context::new(scope, Default::default());
                *cb = Some(v8::Global::new(scope, ctx));
                (ctx, true)
            }
        });
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        if first_file {
            install_natives(scope, global);
        } else {
            // drop app modules + test-file mocks (keep node_modules evals + setup mocks), then
            // wipe per-file framework state (tests/spies/timers/mocks/hoist/stubs/DOM document).
            reset_app_registry();
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

        // process.cwd(): the test's project root (nearest package.json outside node_modules).
        {
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
        let setup_us = setup_start.elapsed().as_secs_f64() * 1_000_000.0;

        let result: Result<TestReport, String> = 'work: {
            // DOM: install once per worker; subsequent files reset it via env.reset() above.
            if needs_dom(entry_abs) && !DOM_INSTALLED.with(|d| d.get()) {
                setup_dom(scope, entry_abs);
                DOM_INSTALLED.with(|d| d.set(true));
            }
            // Setup files run ONCE per worker (vitest isolate:false semantics): their hooks,
            // matchers and mocks persist across files. After the first run we snapshot the hook
            // baseline (restored each file) and the setup-registered mock paths (kept on reset).
            if first_file {
                for sf in setup_files {
                    run_setup_file(scope, sf, externals, mock_targets);
                }
                SETUP_MOCKS.with(|s| {
                    *s.borrow_mut() = REGISTRY.with(|r| r.borrow().mocks.keys().cloned().collect());
                });
                if let Some(code) = v8::String::new(
                    scope,
                    "globalThis.__ttCaptureHookBaseline && globalThis.__ttCaptureHookBaseline();",
                ) {
                    if let Some(s) = v8::Script::compile(scope, code, None) {
                        s.run(scope);
                    }
                }
                snapshot_setup_mocks(scope);
            }
            run_entry_mocks(scope, entry_abs);
            {
                let g = scope.get_current_context().global(scope);
                if let Some(k) = v8::String::new(scope, "__hoistedIdx") {
                    let zero = v8::Integer::new(scope, 0);
                    g.set(scope, k.into(), zero.into());
                }
            }
            if mr_enabled() && esbuild_transform_cjs(entry_abs, false).is_some() {
                let err = {
                    let tc = std::pin::pin!(v8::TryCatch::new(scope));
                    let tc = &mut tc.init();
                    if load_cjs(tc, entry_abs, false).is_none() {
                        Some(if tc.has_caught() {
                            tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_else(|| "entry load failed".into())
                        } else {
                            "entry load failed".to_string()
                        })
                    } else {
                        tc.perform_microtask_checkpoint();
                        None
                    }
                };
                if let Some(e) = err {
                    break 'work Err(e);
                }
                if let Some(d) = entry_abs.parent() {
                    let js = format!("globalThis.__ttDir = {:?};", d.to_string_lossy());
                    if let Some(code) = v8::String::new(scope, &js) {
                        if let Some(s) = v8::Script::compile(scope, code, None) {
                            s.run(scope);
                        }
                    }
                }
                break 'work drive_tests(scope, global);
            }
            let Some(module) = load_graph(scope, load_target) else {
                break 'work Err("graph load failed".to_string());
            };
            {
                let tc = std::pin::pin!(v8::TryCatch::new(scope));
                let tc = &mut tc.init();
                let ok = module.instantiate_module(tc, resolve_callback);
                if ok != Some(true) {
                    let msg = if tc.has_caught() {
                        tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_else(|| "unknown".into())
                    } else {
                        "instantiate failed".to_string()
                    };
                    break 'work Err(format!("instantiate: {msg}"));
                }
            }
            let ev_err: Option<String> = {
                let tc = std::pin::pin!(v8::TryCatch::new(scope));
                let tc = &mut tc.init();
                let ev = module.evaluate(tc);
                tc.perform_microtask_checkpoint();
                if tc.has_caught() {
                    Some(format!("evaluate threw: {}", tc.exception().map(|e| e.to_rust_string_lossy(tc)).unwrap_or_else(|| "unknown".into())))
                } else if ev.is_none() || module.get_status() != v8::ModuleStatus::Evaluated {
                    if module.get_status() == v8::ModuleStatus::Errored {
                        Some(format!("module errored: {}", module.get_exception().to_rust_string_lossy(tc)))
                    } else {
                        Some(format!("evaluate failed: status={:?}", module.get_status()))
                    }
                } else {
                    None
                }
            };
            if let Some(e) = ev_err {
                break 'work Err(e);
            }
            drive_tests(scope, global)
        };

        // NOTE: no clear_registry() here — node_modules Globals stay alive for the next file;
        // app + mock entries are pruned at the top of the next file (reset_app_registry).
        result.map(|mut r| {
            r.setup_us = setup_us;
            r
        })
    })
}

/// End-of-worker cleanup for the reuse path: drop registry Globals + the context while the
/// isolate is still alive, then drop the isolate. Avoids a shutdown assert from handles
/// outliving the isolate. No-op when reuse was never used.
pub fn end_worker_reuse() {
    if REUSE_ISO.with(|i| i.borrow().is_none()) {
        return;
    }
    clear_registry();
    REUSE_CTX.with(|c| {
        let _ = c.borrow_mut().take();
    });
    REUSE_ISO.with(|i| {
        let _ = i.borrow_mut().take();
    });
    DOM_INSTALLED.with(|d| d.set(false));
    SETUP_MOCKS.with(|s| s.borrow_mut().clear());
    SETUP_MOCK_SNAPSHOT.with(|s| s.borrow_mut().clear());
}

/// Whether isolate-reuse was selected for this run (env or vitest config). False until the first
/// non-forced file resolves the decision.
pub fn is_reuse_enabled() -> bool {
    reuse_isolate_enabled()
}

/// Authoritative fresh-isolate retry: run `entry` on a FRESH per-file isolate even under reuse.
/// A file that failed under reuse may have hit a cross-file leak artifact; re-running it in a
/// clean isolate (which is 6189/0 territory) is the source of truth. The worker's reuse REGISTRY
/// is swapped out for the duration so the fresh isolate's Globals don't clobber the persistent
/// reuse module cache (the saved registry's Globals stay valid — the persistent isolate lives on).
pub fn run_test_file_fresh(entry: &Path) -> Result<TestReport, String> {
    let saved = REGISTRY.with(|r| std::mem::take(&mut *r.borrow_mut()));
    FORCE_FRESH.with(|f| f.set(true));
    // Panic-safe: a V8/loader panic must NOT lose the saved reuse registry or strand FORCE_FRESH.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_test_file(entry)));
    FORCE_FRESH.with(|f| f.set(false));
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
) -> Result<TestReport, String> {
    let tt_key = v8::String::new(scope, "__tt").unwrap();
    let tt = v8::Local::<v8::Object>::try_from(
        global.get(scope, tt_key.into()).ok_or("no __tt")?,
    )
    .map_err(|_| "__tt not object")?;
    let run_key = v8::String::new(scope, "run").unwrap();
    let run = v8::Local::<v8::Function>::try_from(tt.get(scope, run_key.into()).ok_or("no run")?)
        .map_err(|_| "run not fn")?;
    let result = run.call(scope, tt.into(), &[]).ok_or("run() threw")?;

    // run() returns a Promise<summary>. Drive the event loop with Node ordering:
    // drain process.nextTick + Promise microtasks to a fixpoint, then run ONE macrotask
    // (timer), and repeat. Between every macrotask the micro/nextTick queues fully drain.
    let promise = v8::Local::<v8::Promise>::try_from(result).map_err(|_| "run() not promise")?;
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > 2_000_000 {
            return Err("event loop did not settle (possible infinite timers)".into());
        }
        loop {
            let ran = call_global_bool(scope, global, "__drainNextTicks");
            scope.perform_microtask_checkpoint();
            let pending = call_global_bool(scope, global, "__hasNextTicks");
            if !ran && !pending {
                break;
            }
        }
        if promise.state() != v8::PromiseState::Pending {
            break;
        }
        if !call_global_bool(scope, global, "__stepMacro") {
            break;
        }
    }
    if promise.state() != v8::PromiseState::Fulfilled {
        let reason = if promise.state() == v8::PromiseState::Rejected {
            format!("rejected: {}", promise.result(scope).to_rust_string_lossy(scope))
        } else {
            "still pending".to_string()
        };
        return Err(format!("test run promise did not fulfill ({reason})"));
    }
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

    let mut failures = Vec::new();
    let fkey = v8::String::new(scope, "failures").unwrap();
    if let Some(arr) = summary
        .get(scope, fkey.into())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
    {
        for i in 0..arr.length() {
            if let Some(v) = arr.get_index(scope, i) {
                failures.push(v.to_rust_string_lossy(scope));
            }
        }
    }
    Ok(TestReport { passed, failed, failures, setup_us: 0.0 })
}

/// Call a no-arg global JS function and coerce its result to bool.
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
}

fn clear_registry() {
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        reg.esm_by_path.clear();
        reg.cjs_synth_by_path.clear();
        reg.cjs_exports.clear();
        reg.real_exports.clear();
        reg.cjs_export_names.clear();
        reg.path_by_hash.clear();
        reg.mocks.clear();
        reg.extra_exports.clear();
        reg.loading.clear();
        reg.lazy_stub_ns.clear();
        reg.import_edges.clear();
    });
}

/// Initialize V8 once for the process.
pub fn init_v8() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // V8 flag tuning for short-lived per-file isolates. Each test file gets a fresh isolate
        // that allocates a burst then dies, so: (a) a larger young generation lets a whole file's
        // allocation live in new-space (fewer scavenges, less promotion), and (b) concurrent
        // marking/sweeping helper threads only add cross-thread overhead when 8 job-threads each
        // spawn their own — the heaps are too small/short-lived to benefit from background GC.
        // Overridable via TURBO_V8_FLAGS (empty string disables the defaults entirely) so the set
        // is A/B-testable without a rebuild. Generic: helps any allocation-heavy short-lived run.
        let flags = std::env::var("TURBO_V8_FLAGS").unwrap_or_default();
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
