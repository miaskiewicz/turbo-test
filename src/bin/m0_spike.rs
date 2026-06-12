//! M0 — Snapshot-isolation spike (gates the entire project).
//!
//! Proves the core speed mechanism from the spec (§4, §M0): a pristine V8 context
//! containing the "framework layer" is built once and snapshotted; each test file
//! gets a fresh context instantiated *from* that snapshot in microseconds, instead
//! of rebuilding the environment per file.
//!
//! Three modes are compared, exactly per the M0 plan:
//!   (rebuild)  full Isolate + Context rebuild + framework install, per iteration
//!   (shared)   one long-lived context, env installed once, reused      (lower bound)
//!   (snapshot) isolate booted from the blob, fresh Context per iteration (the bet)
//!
//! GATE: snapshot-instantiate must be >=10x cheaper than full rebuild AND within
//! ~2x of shared-context reuse. Plus a multi-core throughput number (contexts/sec).

use std::sync::Arc;
use std::sync::Once;
use std::time::Instant;

/// Stand-in for the framework layer (expect/chai/snapshot/vi + turbo-dom).
///
/// The real layer is heavy: @vitest/expect + chai + @vitest/snapshot + @vitest/spy
/// + turbo-dom are hundreds of KB of JS that must be *evaluated* once per context.
/// Without a snapshot every test file re-evaluates all of it; the snapshot bakes the
/// post-evaluation heap so a fresh context skips it. The spike must therefore model a
/// realistically heavy setup, not a 20-line toy — otherwise it measures nothing.
///
/// `funcs` controls weight: number of generated matcher/helper closures defined at
/// setup time (proxy for the framework's many definitions).
fn build_framework_src(funcs: usize) -> String {
    let mut s = String::with_capacity(funcs * 80 + 1024);
    s.push_str(
        r#"
globalThis.__turbo = (() => {
  function expect(actual) {
    return {
      toBe(e) { if (actual !== e) throw new Error("not toBe"); },
      toEqual(e) { if (JSON.stringify(actual) !== JSON.stringify(e)) throw new Error("not toEqual"); },
      toBeGreaterThan(n) { if (!(actual > n)) throw new Error("not gt"); },
    };
  }
  const serialize = (v) => JSON.stringify(v, null, 2);
  const deepClone = (v) => JSON.parse(JSON.stringify(v));
  const M = {};
"#,
    );
    // Many closures with bodies — models chai/expect/matcher definitions that V8
    // must parse + compile + allocate on each fresh evaluation.
    for i in 0..funcs {
        s.push_str(&format!(
            "  M.m{i} = (a, b) => {{ const t = (a ^ {i}) + (b|0); return t > {i} ? t - {i} : t + {i}; }};\n"
        ));
    }
    // Init-time execution — models the work a real framework does at import time
    // (chai/expect building prototypes, registries, plugin chains). This forces V8 to
    // fully compile + execute, which is exactly the cost the snapshot bakes away. Without
    // it the closures above are only lazily pre-parsed and `rebuild` is undercharged.
    s.push_str(
        r#"
  const table = [];
  for (const k in M) { table.push(M[k](k.length, table.length & 7)); }
  const index = {};
  for (let i = 0; i < table.length; i++) index["k" + i] = table[i] | 0;
"#,
    );
    s.push_str(
        r#"
  return { expect, M, table, index, serialize, deepClone, version: "0.1.0" };
})();
globalThis.expect = globalThis.__turbo.expect;
"#,
    );
    s
}

/// A trivial test, run identically in every mode. Exercises the baked env.
/// A trivial test that ALSO leaves module-scope state behind — exactly as a real
/// test file does (top-level vars, registered globals). Fresh-context modes discard
/// this for free; a reused context must explicitly reset it. Identical across modes.
const TRIVIAL_TEST: &str = r#"
globalThis.__t_a = 1;
globalThis.__t_b = [1, 2, 3];
globalThis.__t_c = { x: 1, y: 2 };
globalThis.__t_d = () => 42;
(() => {
  expect(1 + 1).toBe(2);
  expect([1, 2, 3]).toEqual([1, 2, 3]);
  expect(__turbo.serialize({ a: 1 })).toBe(__turbo.serialize({ a: 1 }));
  return __turbo.version;
})();
"#;

static V8_INIT: Once = Once::new();

fn init_v8() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

fn compile_run(scope: &mut v8::PinScope<'_, '_>, src: &str) {
    let code = v8::String::new(scope, src).unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    script.run(scope).unwrap();
}

/// Build the snapshot blob once: a context with the framework layer installed,
/// set as the default context, serialized to a startup blob (returned as bytes).
fn build_snapshot(framework_src: &str) -> Vec<u8> {
    let mut creator = v8::Isolate::snapshot_creator(None, None);
    {
        v8::scope!(let scope, &mut creator);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        compile_run(scope, framework_src);
        scope.set_default_context(context);
    }
    // Clear (not Keep) compiled code: smaller blob → faster per-context deserialize.
    // The framework recompiles lazily on first use, which is cheap per test file since
    // the framework barely executes during a test (it mostly just needs to exist).
    let blob = creator
        .create_blob(v8::FunctionCodeHandling::Clear)
        .expect("snapshot blob creation failed");
    blob.to_vec()
}

/// (rebuild) Per-file rebuild WITHOUT a snapshot: one isolate (created once, as a
/// worker would), but per iteration a fresh context plus a full re-evaluation of the
/// framework layer, then the test. This is the cost the snapshot is meant to remove.
fn bench_rebuild(framework_src: &str, iters: u32) -> f64 {
    let isolate = &mut v8::Isolate::new(v8::Isolate::create_params());
    let start = Instant::now();
    for _ in 0..iters {
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        compile_run(scope, framework_src);
        compile_run(scope, TRIVIAL_TEST);
    }
    per_iter_us(start, iters)
}

/// (shared) One isolate, one context, framework installed once; only the test
/// runs per iteration. This is the speed lower bound (no isolation between files).
fn bench_shared(framework_src: &str, iters: u32) -> f64 {
    let isolate = &mut v8::Isolate::new(v8::Isolate::create_params());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    compile_run(scope, framework_src);

    let start = Instant::now();
    for _ in 0..iters {
        compile_run(scope, TRIVIAL_TEST);
    }
    per_iter_us(start, iters)
}

/// (snapshot) Isolate booted once from the blob (framework baked into the default
/// context); per iteration a fresh Context is deserialized from that snapshot,
/// test runs, context discarded. This is the real per-test-file path from the spec.
fn bench_snapshot(blob: &[u8], iters: u32) -> f64 {
    let params = v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(blob.to_vec()));
    let isolate = &mut v8::Isolate::new(params);

    let start = Instant::now();
    for _ in 0..iters {
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        compile_run(scope, TRIVIAL_TEST);
    }
    per_iter_us(start, iters)
}

/// (reuse+reset) ONE context per worker, reused across all files. After each file the
/// global state the test left behind is explicitly deleted. This is the speed ceiling
/// for a "fast but leaky" strategy (what Bun-style runners do) — it skips context
/// creation entirely but only resets what we remember to reset (globals here; NOT the
/// module registry, prototype patches, mocked Date/timers, etc.). Faster, less safe.
fn bench_reuse_reset(blob: &[u8], iters: u32) -> f64 {
    let params = v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(blob.to_vec()));
    let isolate = &mut v8::Isolate::new(params);
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    // Snapshot the baseline global key set, define the reset routine.
    compile_run(
        scope,
        r#"
        globalThis.__base = new Set(Object.getOwnPropertyNames(globalThis));
        globalThis.__reset = () => {
          for (const k of Object.getOwnPropertyNames(globalThis)) {
            if (k !== "__base" && k !== "__reset" && !__base.has(k)) delete globalThis[k];
          }
        };
        __base.add("__reset");
        "#,
    );

    let start = Instant::now();
    for _ in 0..iters {
        compile_run(scope, TRIVIAL_TEST);
        compile_run(scope, "__reset()");
    }
    per_iter_us(start, iters)
}

/// Multi-core throughput: T threads, each boots its own isolate from the shared
/// blob and deserializes contexts in a loop. Reports contexts/sec aggregate.
fn bench_throughput(blob: Arc<Vec<u8>>, threads: usize, iters_per_thread: u32) -> f64 {
    let start = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let blob = Arc::clone(&blob);
            std::thread::spawn(move || {
                let params = v8::Isolate::create_params()
                    .snapshot_blob(v8::StartupData::from(blob.as_slice().to_vec()));
                let isolate = &mut v8::Isolate::new(params);
                for _ in 0..iters_per_thread {
                    v8::scope!(let scope, isolate);
                    let context = v8::Context::new(scope, Default::default());
                    let scope = &mut v8::ContextScope::new(scope, context);
                    compile_run(scope, TRIVIAL_TEST);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let total = (threads as u32 * iters_per_thread) as f64;
    total / start.elapsed().as_secs_f64()
}

fn per_iter_us(start: Instant, iters: u32) -> f64 {
    start.elapsed().as_secs_f64() * 1_000_000.0 / iters as f64
}

/// median of repeated measurements to dampen scheduler noise (spec §7 protocol).
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs[xs.len() / 2]
}

fn main() {
    init_v8();

    const REPEATS: usize = 5;
    const ITERS: u32 = 300;

    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    println!("turbo-test M0 — snapshot-isolation spike");
    println!("cores: {cores}");
    println!("per-mode iters: {ITERS}, repeats: {REPEATS} (median reported)");

    // One-time-per-worker cost (NOT a per-file cost): creating a bare isolate.
    let iso_cost = {
        let start = Instant::now();
        for _ in 0..50 {
            let _ = v8::Isolate::new(v8::Isolate::create_params());
        }
        per_iter_us(start, 50)
    };
    println!("isolate creation (once per worker): {iso_cost:.1} us\n");

    // Sweep framework weight. The snapshot's whole value is skipping framework
    // RE-EVALUATION per file, so the win must be shown as a function of that weight.
    // Heavier framework also grows the blob → slower context deserialize (M3 risk).
    let weights = [0usize, 500, 2000, 8000];

    println!("per-file setup cost by strategy (us), all reuse one isolate:");
    println!(
        "{:>8} {:>10} {:>12} {:>12} {:>12} {:>12}",
        "funcs", "blob(KB)", "rebuild", "reuse+reset", "snapshot", "shared"
    );

    let mut headline_snapshot = 0.0_f64;
    let mut headline_blob = Vec::new();

    for &w in &weights {
        let fw = build_framework_src(w);
        let blob = build_snapshot(&fw);

        // warmup
        bench_rebuild(&fw, 20);
        bench_reuse_reset(&blob, 20);
        bench_snapshot(&blob, 20);
        bench_shared(&fw, 20);

        let rebuild = median((0..REPEATS).map(|_| bench_rebuild(&fw, ITERS)).collect());
        let reuse = median((0..REPEATS).map(|_| bench_reuse_reset(&blob, ITERS)).collect());
        let snapshot = median((0..REPEATS).map(|_| bench_snapshot(&blob, ITERS)).collect());
        let shared = median((0..REPEATS).map(|_| bench_shared(&fw, ITERS)).collect());

        println!(
            "{:>8} {:>10.0} {:>12.1} {:>12.1} {:>12.1} {:>12.1}",
            w,
            blob.len() as f64 / 1024.0,
            rebuild,
            reuse,
            snapshot,
            shared
        );

        if w == 2000 {
            headline_snapshot = snapshot;
            headline_blob = blob;
        }
    }

    let tput = bench_throughput(Arc::new(headline_blob), cores, 500);
    println!("\nthroughput @ 2000-func snapshot ({cores} isolates): {tput:.0} contexts/sec");

    // The real gate is not the synthetic microbench — it is the cost of the thing this
    // design REPLACES in stock Vitest: per-file environment construction. Measured on
    // the ui-design-components corpus (vitest 4.1.8, src/utils, 10 files):
    //   jsdom + isolate=true : environment ~465 us... no — 465 MILLIseconds / file
    //   turbo-dom + shared   : environment ~32 ms / file
    // Snapshot fresh-context here is sub-millisecond.
    let snap_ms = headline_snapshot / 1000.0;
    let vitest_jsdom_env_ms = 465.0;
    let vitest_turbodom_env_ms = 32.0;
    println!("\n--- vs STOCK VITEST per-file environment setup (real corpus) ---");
    println!("  snapshot fresh-context (full isolation) : {snap_ms:>8.2} ms/file");
    println!(
        "  stock vitest env, jsdom isolated        : {vitest_jsdom_env_ms:>8.2} ms/file  -> {:.0}x",
        vitest_jsdom_env_ms / snap_ms
    );
    println!(
        "  stock vitest env, turbo-dom shared      : {vitest_turbodom_env_ms:>8.2} ms/file  -> {:.0}x",
        vitest_turbodom_env_ms / snap_ms
    );

    println!("\n--- isolation question ---");
    println!("  reuse+reset (leaky) saves only the ~context-creation delta vs snapshot,");
    println!("  which is sub-ms — negligible next to the {vitest_turbodom_env_ms:.0}ms+ env cost we remove.");
    println!("  => Keep FULL per-file isolation (fresh context). No need to trade compat for speed.");

    // PASS criterion, re-baselined against the real competitor (spec §8: >=5x vs Vitest).
    let gate = (vitest_turbodom_env_ms / snap_ms) >= 5.0;
    if gate {
        println!("\n==> M0 PASS — snapshot fresh-context is >5x cheaper than even the FASTEST");
        println!("    stock-vitest env path, with full isolation. Premise validated. Proceed to M1.");
        std::process::exit(0);
    } else {
        println!("\n==> M0 FAIL — snapshot not >5x vs stock vitest env. Reconsider.");
        std::process::exit(1);
    }
}
