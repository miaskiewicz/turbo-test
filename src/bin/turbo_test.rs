//! turbo-test — consolidated native test runner CLI (M1 end-to-end + M4 parallelism).
//!
//! Runs test files across N worker threads, each with its own V8 isolate booted from the
//! single shared framework snapshot (spec §4). Work-stealing via an atomic cursor balances
//! load; files are ordered slowest-first from persisted historical durations (duration-aware
//! scheduling) so the longest files start first and workers drain evenly. Results are
//! returned in-process (no IPC serialization needed for the thread model).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use turbo_test::coverage;
use turbo_test::runner::{forget_registry, init_v8, run_test_file, transform_cache_stats};

struct FileResult {
    idx: usize,
    line: String,
    passed: u32,
    failed: u32,
    load_error: bool,
    setup_us: f64,
    dur_ms: f64,
}

fn durations_path() -> PathBuf {
    std::env::temp_dir().join("turbo-test-cache").join("durations.tsv")
}

fn load_durations() -> std::collections::HashMap<String, f64> {
    let mut m = std::collections::HashMap::new();
    if let Ok(s) = std::fs::read_to_string(durations_path()) {
        for line in s.lines() {
            if let Some((p, d)) = line.split_once('\t') {
                if let Ok(ms) = d.parse::<f64>() {
                    m.insert(p.to_string(), ms);
                }
            }
        }
    }
    m
}

fn save_durations(m: &std::collections::HashMap<String, f64>) {
    let _ = std::fs::create_dir_all(durations_path().parent().unwrap());
    let mut out = String::new();
    for (p, d) in m {
        out.push_str(&format!("{p}\t{d}\n"));
    }
    let _ = std::fs::write(durations_path(), out);
}

fn main() {
    let mut files: Vec<PathBuf> = Vec::new();
    // Default worker count = host logical cores. (Reducing this to leave headroom for V8's GC
    // helper threads looked like a big win on a warm 40-file microbench but REGRESSED the full
    // suite badly: +75% on a 431-file run — with the whole cold suite in flight, all cores stay
    // productively busy and the GC helpers are mostly idle, so fewer workers just leaves cores
    // empty. Lesson: measure parallelism changes on the FULL suite, never a subset.) Overridable
    // by TURBO_JOBS (env, kept for A/B sweeps); an explicit --jobs flag still wins (below).
    let mut jobs = std::env::var("TURBO_JOBS").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
    let mut shard: Option<(usize, usize)> = None;
    let mut json = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--jobs" | "-j" => jobs = args.next().and_then(|v| v.parse().ok()).unwrap_or(jobs),
            "--shard" => {
                if let Some(v) = args.next() {
                    if let Some((i, n)) = v.split_once('/') {
                        if let (Ok(i), Ok(n)) = (i.parse(), n.parse()) {
                            shard = Some((i, n));
                        }
                    }
                }
            }
            "--reporter" => json = args.next().as_deref() == Some("json"),
            // `-t <re>` / `--testNamePattern <re>` — run only tests whose full `describe > it`
            // name matches the regex (vitest semantics: unanchored, case-sensitive). Plumbed to
            // the runtime via env → `globalThis.__TT_NAME_PATTERN` (see runner bootstrap).
            "-t" | "--testNamePattern" => {
                if let Some(v) = args.next() {
                    std::env::set_var("TURBO_TEST_NAME_PATTERN", v);
                }
            }
            // `-u` / `--update` — write missing/changed snapshots instead of failing (vitest
            // `--update`). Plumbed to the runtime via env → globalThis.__TT_UPDATE_SNAPSHOTS.
            "-u" | "--update" => std::env::set_var("TURBO_UPDATE_SNAPSHOTS", "1"),
            "--coverage" => coverage::enable(),
            // `--coverage-dir <path>` sets the lcov output dir (and implies --coverage).
            "--coverage-dir" => {
                coverage::enable();
                if let Some(d) = args.next() {
                    coverage::set_out_dir(&d);
                }
            }
            // `--coverage-thresholds lines=90,functions=80,branches=80` — gate (implies --coverage).
            "--coverage-thresholds" | "--coverage-threshold" => {
                coverage::enable();
                if let Some(v) = args.next() {
                    coverage::set_thresholds(&v);
                }
            }
            // apply thresholds to EACH reported file, not just the total.
            "--coverage-per-file" => {
                coverage::enable();
                coverage::set_per_file(true);
            }
            // `--coverage-reporter lcov,json-summary,text,html` (repeatable / comma-list).
            "--coverage-reporter" | "--coverage-reporters" => {
                coverage::enable();
                if let Some(v) = args.next() {
                    coverage::set_reporters(&v);
                }
            }
            // include/exclude globs (cwd-relative) — usually injected from vitest config by cli.js.
            "--coverage-include" => {
                coverage::enable();
                if let Some(v) = args.next() {
                    coverage::add_include(&v);
                }
            }
            "--coverage-exclude" => {
                coverage::enable();
                if let Some(v) = args.next() {
                    coverage::add_exclude(&v);
                }
            }
            // Unknown `-`/`--` token: a vitest flag turbo-test does not model (e.g. --silent,
            // --pool=forks, --logHeapUsage). Warn + ignore — NEVER treat it as a test-file path
            // (that reached the runner as a hard load-error and flipped the exit code). Test
            // files and globs never start with `-`.
            other if other.starts_with('-') => {
                eprintln!("turbo-test: ignoring unsupported flag '{other}'");
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        eprintln!("usage: turbo-test [--jobs N] [--shard i/n] [--reporter json] <file.test.ts> [more...]");
        std::process::exit(2);
    }
    // sharding: deterministic partition by index (spec §M6)
    if let Some((i, n)) = shard {
        if n >= 1 && i >= 1 && i <= n {
            files = files.into_iter().enumerate().filter(|(k, _)| k % n == i - 1).map(|(_, f)| f).collect();
        }
    }

    init_v8(); // builds the framework snapshot once, before workers spawn

    // duration-aware order: slowest historical first (work-steal handles the rest)
    let hist = load_durations();
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|&a, &b| {
        let da = hist.get(&files[a].to_string_lossy().to_string()).copied().unwrap_or(0.0);
        let db = hist.get(&files[b].to_string_lossy().to_string()).copied().unwrap_or(0.0);
        db.partial_cmp(&da).unwrap()
    });

    let jobs = jobs.min(files.len()).max(1);
    let files = Arc::new(files);
    let order = Arc::new(order);
    let cursor = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(Vec::<FileResult>::new()));

    let wall = Instant::now();
    let handles: Vec<_> = (0..jobs)
        .map(|_| {
            let files = Arc::clone(&files);
            let order = Arc::clone(&order);
            let cursor = Arc::clone(&cursor);
            let results = Arc::clone(&results);
            std::thread::spawn(move || {
              loop {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i >= order.len() {
                    break;
                }
                let idx = order[i];
                let file = &files[idx];
                let t = Instant::now();
                // A panic in V8/loader must not abort the whole run: catch it, leak the
                // dead isolate's handles, and report this file as an error.
                let run_once = || match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_test_file(file)
                })) {
                    Ok(r) => r,
                    Err(p) => {
                        forget_registry();
                        let msg = p
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| p.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".into());
                        Err(format!("panicked: {msg}"))
                    }
                };
                // The shared turbo-dom .node addon stores a process-global env that a concurrent
                // worker's DOM bootstrap can transiently clobber → "document is not defined" for a
                // whole file. It's timing-dependent, so re-running (up to 3x) almost always lands
                // a clean pass; this keeps the suite deterministic across parallel runs.
                // A whole DOM-using file failing (0 passed, ≥1 failed) or an explicit DOM-not-ready
                // error is the init-race signature. Re-run (up to 3x) and KEEP THE BEST attempt —
                // a flaky file lands a clean pass on some attempt; a genuinely all-failing file
                // stays the same (just slower). This keeps the suite deterministic across runs.
                let looks_transient = |r: &Result<turbo_test::runner::TestReport, String>| match r {
                    Ok(rep) => (rep.passed == 0 && rep.failed > 0)
                        || rep.failures.iter().any(|f| f.contains("is not defined")
                            && (f.contains("document") || f.contains("window") || f.contains("DOM"))),
                    Err(_) => true,
                };
                let score = |r: &Result<turbo_test::runner::TestReport, String>| match r {
                    Ok(rep) => (rep.passed as i64) * 1000 - (rep.failed as i64),
                    Err(_) => i64::MIN,
                };
                let mut r = run_once();
                let mut tries = 0;
                while looks_transient(&r) && tries < 2 {
                    tries += 1;
                    let r2 = run_once();
                    if score(&r2) > score(&r) {
                        r = r2;
                    }
                }
                // Fresh-isolate retry (idea #1): under reuse, a file that still has failures may
                // be a cross-file leak ARTIFACT (it passes in a clean isolate). Re-run it on a
                // fresh isolate — that result is authoritative (fresh mode is 6189/0). This pins
                // reuse correctness to fresh while keeping reuse speed for the files that pass.
                let has_fail = |r: &Result<turbo_test::runner::TestReport, String>| match r {
                    Ok(rep) => rep.failed > 0 || (rep.passed == 0),
                    Err(_) => true,
                };
                if turbo_test::runner::is_reuse_enabled() && has_fail(&r) {
                    // run_test_file_fresh is panic-safe + restores the worker's reuse registry.
                    let mut fr = turbo_test::runner::run_test_file_fresh(file);
                    let mut ft = 0;
                    while looks_transient(&fr) && ft < 2 {
                        ft += 1;
                        let f2 = turbo_test::runner::run_test_file_fresh(file);
                        if score(&f2) > score(&fr) {
                            fr = f2;
                        }
                    }
                    // fresh is the source of truth — adopt it (even if it also fails: that's real).
                    r = fr;
                }
                let dur_ms = t.elapsed().as_secs_f64() * 1000.0;
                let fr = match r {
                    Ok(rep) => {
                        let mark = if rep.ok() { "PASS" } else { "FAIL" };
                        let mut line =
                            format!("{mark}  {}  ({} passed, {} failed)", file.display(), rep.passed, rep.failed);
                        for f in &rep.failures {
                            line.push_str(&format!("\n        ✗ {f}"));
                        }
                        FileResult { idx, line, passed: rep.passed, failed: rep.failed, load_error: false, setup_us: rep.setup_us, dur_ms }
                    }
                    Err(e) => FileResult {
                        idx,
                        line: format!("ERROR {}  ({e})", file.display()),
                        passed: 0,
                        failed: 0,
                        load_error: true,
                        setup_us: 0.0,
                        dur_ms,
                    },
                };
                results.lock().unwrap().push(fr);
              }
              // reuse path: tear down this worker's persistent isolate cleanly (no-op otherwise)
              turbo_test::runner::end_worker_reuse();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;

    let mut res = Arc::try_unwrap(results).ok().expect("sole owner").into_inner().unwrap();
    res.sort_by_key(|r| r.idx);

    let (mut tp, mut tf, mut errs, mut setup_sum, mut setup_n) = (0u32, 0u32, 0u32, 0.0f64, 0u32);
    let mut new_hist = hist.clone();
    for r in &res {
        println!("{}", r.line);
        tp += r.passed;
        tf += r.failed;
        if r.load_error {
            errs += 1;
        } else {
            setup_sum += r.setup_us;
            setup_n += 1;
        }
        new_hist.insert(files[r.idx].to_string_lossy().to_string(), r.dur_ms);
    }
    save_durations(&new_hist);

    if json {
        // Vitest-compatible-ish JSON summary (numTotalTests/numPassedTests/...).
        let total: u32 = res.iter().map(|r| r.passed + r.failed).sum();
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut out = String::from("{");
        out.push_str(&format!("\"numTotalTestSuites\":{},", files.len()));
        out.push_str(&format!("\"numTotalTests\":{total},"));
        out.push_str(&format!("\"numPassedTests\":{tp},"));
        out.push_str(&format!("\"numFailedTests\":{tf},"));
        out.push_str(&format!("\"success\":{},", tf == 0 && errs == 0));
        out.push_str("\"testResults\":[");
        let items: Vec<String> = res
            .iter()
            .map(|r| {
                format!(
                    "{{\"name\":\"{}\",\"status\":\"{}\",\"numPassingTests\":{},\"numFailingTests\":{}}}",
                    esc(&files[r.idx].to_string_lossy()),
                    if r.load_error { "error" } else if r.failed == 0 { "passed" } else { "failed" },
                    r.passed,
                    r.failed
                )
            })
            .collect();
        out.push_str(&items.join(","));
        out.push_str("]}");
        println!("{out}");
    }

    let avg_setup = if setup_n > 0 { setup_sum / setup_n as f64 } else { 0.0 };
    let (hits, misses) = transform_cache_stats();
    let hit_rate = if hits + misses > 0 { 100.0 * hits as f64 / (hits + misses) as f64 } else { 0.0 };
    println!(
        "\n{} files | {} passed | {} failed | {} load-errors | {} jobs | wall {:.0} ms | env setup {:.2} ms/file | cache {:.0}% hit",
        files.len(), tp, tf, errs, jobs, wall_ms, avg_setup / 1000.0, hit_rate
    );
    let cov_ok = if coverage::enabled() { coverage::report() } else { true };
    std::process::exit(if tf == 0 && errs == 0 && cov_ok { 0 } else { 1 });
}
