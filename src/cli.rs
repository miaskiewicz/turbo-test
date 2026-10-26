//! The `turbo-test` CLI driver — the body of `src/bin/turbo_test.rs`, split into testable pieces.
//!
//! Runs test files across N worker threads, each with its own V8 isolate booted from the
//! single shared framework snapshot (spec §4). Work-stealing via an atomic cursor balances
//! load; files are ordered slowest-first from persisted historical durations (duration-aware
//! scheduling) so the longest files start first and workers drain evenly. Results are
//! returned in-process (no IPC serialization needed for the thread model).
//!
//! Layout: [`parse_args`] turns the launcher's effective argv into [`Options`] (no side effects),
//! [`execute_file`] runs one file with the panic guard + transient/fresh-isolate retries against an
//! injectable [`Engine`], [`run_files`] is the worker pool, and the `render_*` / [`footer`] /
//! [`summary`] functions are the pure reporters. [`run`] glues them together and returns the
//! process exit code; the binary is a thin `exit(main_with_args(argv))` wrapper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::coverage;
use crate::runner::{self, TestCase, TestReport};

/// What running one test file produced: a report, or a load error message.
pub type FileOutcome = Result<TestReport, String>;

/// Printed (to stderr, exit 2) when no test files remain after argument parsing.
pub const USAGE: &str =
    "usage: turbo-test [--jobs N] [--shard i/n] [--reporter json] <file.test.ts> [more...]";

/// One test file's outcome, as the reporters consume it.
#[derive(Debug, Clone)]
pub struct FileResult {
    /// Index into the (sharded) file list.
    pub idx: usize,
    /// The default reporter's `PASS`/`FAIL`/`ERROR` line (with `✗ failure` continuation lines).
    pub line: String,
    pub passed: u32,
    pub failed: u32,
    pub load_error: bool,
    /// The file failed: it failed to load, a test failed, or a suite / file-level error was
    /// reported (a throwing `beforeAll` / `afterAll`, an empty `describe`) with no failed test.
    pub failed_file: bool,
    /// Load-error message (for the synthetic JUnit/TAP failing case); empty otherwise.
    pub error_msg: String,
    /// Per-test cases (name/status/duration/message) for the per-testcase reporters.
    pub tests: Vec<TestCase>,
    /// One state per `describe` block ("pass" / "fail" / "skip" / "todo"), for the JSON counts.
    pub suites: Vec<String>,
    /// The file's snapshot counts, for the JSON report's `snapshot` summary.
    pub snapshot: Option<runner::SnapshotResult>,
    pub setup_us: f64,
    pub dur_ms: f64,
}

/// Which reporter to emit. Anything turbo-test does not implement is accepted-and-ignored and
/// falls back to `Default` (the text reporter) — never an error (vitest-compat requirement).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reporter {
    Default,
    Dot,
    Json,
    Junit,
    Tap,
    Verbose,
}

impl Reporter {
    /// The reporter named by a `--reporter` value (`None` = the flag had no value).
    pub fn parse(v: Option<&str>) -> Reporter {
        match v {
            Some("json") => Reporter::Json,
            Some("junit") => Reporter::Junit,
            Some("tap") => Reporter::Tap,
            Some("verbose") => Reporter::Verbose,
            Some("dot") => Reporter::Dot,
            _ => Reporter::Default,
        }
    }

    /// `json/junit/tap` are machine artifacts — stdout carries only the artifact (the human
    /// summary line goes to stderr). `default/dot/verbose` are human reporters (stdout).
    pub fn is_artifact(self) -> bool {
        matches!(self, Reporter::Json | Reporter::Junit | Reporter::Tap)
    }
}

/// A coverage-configuration call requested by a `--coverage*` flag, applied in argv order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverageOp {
    Enable,
    OutDir(String),
    Thresholds(String),
    PerFile,
    Reporters(String),
    Include(String),
    Exclude(String),
}

impl CoverageOp {
    /// Apply this op to the process-global coverage configuration.
    pub fn apply(&self) {
        match self {
            CoverageOp::Enable => coverage::enable(),
            CoverageOp::OutDir(d) => coverage::set_out_dir(d),
            CoverageOp::Thresholds(v) => coverage::set_thresholds(v),
            CoverageOp::PerFile => coverage::set_per_file(true),
            CoverageOp::Reporters(v) => coverage::set_reporters(v),
            CoverageOp::Include(v) => coverage::add_include(v),
            CoverageOp::Exclude(v) => coverage::add_exclude(v),
        }
    }
}

/// The runner-side options parsed from the launcher's effective argv.
#[derive(Debug, PartialEq)]
pub struct Options {
    pub files: Vec<PathBuf>,
    pub jobs: usize,
    pub shard: Option<(usize, usize)>,
    pub reporter: Reporter,
    pub output_file: Option<String>,
    /// `--bail N`: stop pulling new files once cumulative failed tests >= N (0 = disabled).
    pub bail: usize,
    /// `--allowOnly` / `--no-allowOnly` (the last one given); `None` is vitest's default,
    /// `!isCI` (`crate::ci`): a `.only` fails on CI (turbo-test#75).
    pub allow_only: Option<bool>,
    /// Env vars the runtime reads (`TURBO_TEST_TIMEOUT`, …), set in this order before the run.
    pub env: Vec<(&'static str, String)>,
    /// Coverage configuration calls, applied in this order before the run.
    pub coverage: Vec<CoverageOp>,
    /// `ignoring unsupported flag` warnings, printed to stderr in this order.
    pub warnings: Vec<String>,
}

/// Default worker count = `TURBO_JOBS` when it parses, else host logical cores. (Reducing this
/// to leave headroom for V8's GC helper threads looked like a big win on a warm 40-file
/// microbench but REGRESSED the full suite badly: +75% on a 431-file run — with the whole cold
/// suite in flight, all cores stay productively busy and the GC helpers are mostly idle, so fewer
/// workers just leaves cores empty. Lesson: measure parallelism changes on the FULL suite, never a
/// subset.) An explicit `--jobs` flag still wins (see [`parse_args`]).
pub fn default_jobs(turbo_jobs: Option<&str>) -> usize {
    turbo_jobs.and_then(|v| v.parse().ok()).unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

/// Value-taking vitest flags with no effect on turbo-test (their value is consumed and dropped).
const IGNORED_VALUE_FLAGS: [&str; 5] = [
    "--pool",
    "--mode",
    "--project",
    "--exclude",
    "--maxConcurrency",
];

/// Whether `key` (the part of a `--key=value` token before the first `=`) is a runner value flag,
/// so the token is its inline form rather than an unknown flag.
fn takes_inline_value(key: &str) -> bool {
    key.starts_with("--")
        && (crate::launcher::VALUE_FLAGS.contains(&key) || IGNORED_VALUE_FLAGS.contains(&key))
}

/// Parse the runner flags + test files (the launcher already consumed its own flags and resolved
/// the file set). Pure: env vars, coverage calls and warnings are returned, not applied.
pub fn parse_args<I: IntoIterator<Item = String>>(argv: I, default_jobs: usize) -> Options {
    let mut o = Options {
        files: Vec::new(),
        jobs: default_jobs,
        shard: None,
        reporter: Reporter::Default,
        output_file: None,
        bail: 0,
        allow_only: None,
        env: Vec::new(),
        coverage: Vec::new(),
        warnings: Vec::new(),
    };
    let mut args = argv.into_iter();
    while let Some(a) = args.next() {
        // `--flag=value` (vitest/cac accept it for every value flag) is the one-token form of
        // `--flag value`: split it so both forms share one arm. The inline value is the WHOLE
        // value (`--testNamePattern=a=b` → `a=b`) and the next token is never consumed for it.
        let (key, mut inline) = match a.split_once('=') {
            Some((k, v)) if takes_inline_value(k) => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = || inline.take().or_else(|| args.next());
        match key.as_str() {
            // `--maxWorkers N` is vitest's name for the worker count → maps to --jobs.
            "--jobs" | "-j" | "--maxWorkers" => {
                o.jobs = value().and_then(|v| v.parse().ok()).unwrap_or(o.jobs)
            }
            // turbo-test has no minimum-worker concept; accept-and-ignore the value (documented).
            "--minWorkers" => {
                let _ = value();
            }
            // `--testTimeout N` → default per-test timeout (ms); runtime reads __TT_DEFAULT_TIMEOUT.
            "--testTimeout" => {
                if let Some(v) = value() {
                    if v.parse::<u64>().is_ok() {
                        o.env.push(("TURBO_TEST_TIMEOUT", v));
                    }
                }
            }
            // `--retry N` → global default retry count; runtime reads __TT_DEFAULT_RETRY.
            "--retry" => {
                if let Some(v) = value() {
                    if v.parse::<u64>().is_ok() {
                        o.env.push(("TURBO_TEST_RETRY", v));
                    }
                }
            }
            // `--bail N` → stop the run after N failed tests total (cross-worker, file-granular).
            "--bail" => o.bail = value().and_then(|v| v.parse().ok()).unwrap_or(0),
            // `--silent` → suppress test console.* output; runtime reads __TT_SILENT.
            "--silent" => o.env.push(("TURBO_TEST_SILENT", "1".to_string())),
            // `--allowOnly` / `--no-allowOnly` → permit or forbid `.only` tests (default: !isCI).
            "--allowOnly" => o.allow_only = Some(true),
            "--no-allowOnly" => o.allow_only = Some(false),
            "--shard" => {
                if let Some(v) = value() {
                    if let Some((i, n)) = v.split_once('/') {
                        if let (Ok(i), Ok(n)) = (i.parse(), n.parse()) {
                            o.shard = Some((i, n));
                        }
                    }
                }
            }
            // `--reporter <r>`: select an output reporter. Implemented: default, dot, json,
            // junit, tap, verbose. Any other value (html, tap-flat, hanging-process, …) is
            // accepted-and-ignored → falls back to the text reporter (never an error).
            "--reporter" | "--reporters" => o.reporter = Reporter::parse(value().as_deref()),
            // `--outputFile <path>`: write the active reporter's output to a file (vitest writes
            // the reporter artifact here — JUnit XML, TAP, JSON). Text/dot/verbose still print.
            "--outputFile" | "--output-file" => o.output_file = value(),
            // `-t <re>` / `--testNamePattern <re>` — run only tests whose full `describe > it`
            // name matches the regex (vitest semantics: unanchored, case-sensitive). Plumbed to
            // the runtime via env → `globalThis.__TT_NAME_PATTERN` (see runner bootstrap).
            "-t" | "--testNamePattern" => {
                if let Some(v) = value() {
                    o.env.push(("TURBO_TEST_NAME_PATTERN", v));
                }
            }
            // `--includeTaskLocation`: record each test's location (`location` in the JSON report).
            "--includeTaskLocation" => o.env.push(("TURBO_TASK_LOCATION", "1".to_string())),
            // `-u` / `--update` — write missing/changed snapshots instead of failing (vitest
            // `--update`). Plumbed to the runtime via env → globalThis.__TT_UPDATE_SNAPSHOTS.
            "-u" | "--update" => o.env.push(("TURBO_UPDATE_SNAPSHOTS", "1".to_string())),
            "--coverage" => o.coverage.push(CoverageOp::Enable),
            // `--coverage-dir <path>` sets the lcov output dir (and implies --coverage).
            "--coverage-dir" => {
                o.coverage.push(CoverageOp::Enable);
                if let Some(d) = value() {
                    o.coverage.push(CoverageOp::OutDir(d));
                }
            }
            // `--coverage-thresholds lines=90,functions=80,branches=80` — gate (implies --coverage).
            "--coverage-thresholds" | "--coverage-threshold" => {
                o.coverage.push(CoverageOp::Enable);
                if let Some(v) = value() {
                    o.coverage.push(CoverageOp::Thresholds(v));
                }
            }
            // apply thresholds to EACH reported file, not just the total.
            "--coverage-per-file" => {
                o.coverage.push(CoverageOp::Enable);
                o.coverage.push(CoverageOp::PerFile);
            }
            // `--coverage-reporter lcov,json-summary,text,html` (repeatable / comma-list).
            "--coverage-reporter" | "--coverage-reporters" => {
                o.coverage.push(CoverageOp::Enable);
                if let Some(v) = value() {
                    o.coverage.push(CoverageOp::Reporters(v));
                }
            }
            // include/exclude globs (cwd-relative) — usually injected from vitest config by the
            // launcher.
            "--coverage-include" => {
                o.coverage.push(CoverageOp::Enable);
                if let Some(v) = value() {
                    o.coverage.push(CoverageOp::Include(v));
                }
            }
            "--coverage-exclude" => {
                o.coverage.push(CoverageOp::Enable);
                if let Some(v) = value() {
                    o.coverage.push(CoverageOp::Exclude(v));
                }
            }
            // `--no-file-parallelism` (vitest: run test files serially) → jobs = 1.
            "--no-file-parallelism" => o.jobs = 1,
            // Value-taking vitest flags with no effect on a native single-run runner. Consume the
            // value (inline, or the following token — the launcher forwards flag+value together)
            // so it isn't mistaken for a test-file path.
            "--pool" | "--mode" | "--project" | "--exclude" | "--maxConcurrency" => {
                let _ = value();
            }
            // Boolean / no-effect vitest flags — silently accepted (NOT warned) so scripts and CI
            // passing them stay quiet.
            "--run"
            | "--watch"
            | "-w"
            | "--no-watch"
            | "--logHeapUsage"
            | "--hideSkippedTests"
            | "--no-color"
            | "--color"
            | "--dom"
            | "--segfaultRetry"
            | "--printConsoleTrace"
            | "--disableConsoleIntercept" => {}
            _ if a.starts_with("--sequence")
                || a.starts_with("--inspect")
                || a.starts_with("--browser") => {}
            // Unknown `-`/`--` token: a vitest flag turbo-test does not model. Warn + ignore —
            // NEVER treat it as a test-file path (that reached the runner as a hard load-error and
            // flipped the exit code). Test files and globs never start with `-`.
            other if other.starts_with('-') => o
                .warnings
                .push(format!("turbo-test: ignoring unsupported flag '{other}'")),
            _ => o.files.push(PathBuf::from(a)),
        }
    }
    o
}

/// How the reports name each file: relative to `root` (`--root`, the launcher's `TURBO_ROOT`), as
/// vitest names files relative to its root; without one, or for a file outside it, as given (the
/// launcher names discovered files relative to the working directory).
pub fn display_names(files: &[PathBuf], root: Option<&Path>) -> Vec<PathBuf> {
    files
        .iter()
        .map(|f| {
            root.and_then(|r| {
                std::path::absolute(f)
                    .ok()?
                    .strip_prefix(std::path::absolute(r).ok()?)
                    .ok()
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| f.clone())
        })
        .collect()
}

/// Sharding: deterministic partition by index (spec §M6). An out-of-range `i/n` keeps every file.
pub fn apply_shard(files: Vec<PathBuf>, shard: Option<(usize, usize)>) -> Vec<PathBuf> {
    if let Some((i, n)) = shard {
        if n >= 1 && i >= 1 && i <= n {
            return files
                .into_iter()
                .enumerate()
                .filter(|(k, _)| k % n == i - 1)
                .map(|(_, f)| f)
                .collect();
        }
    }
    files
}

/// Where the per-file duration history lives (`<tmp>/turbo-test-cache/durations.tsv`).
pub fn durations_path() -> PathBuf {
    std::env::temp_dir()
        .join("turbo-test-cache")
        .join("durations.tsv")
}

/// Load the `path\tms` duration history (missing file / malformed lines → ignored). `f64` parsing
/// accepts `NaN`/`inf`; such entries are malformed too (a hand-edited or corrupt cache) → ignored.
pub fn load_durations(path: &Path) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(path) {
        for line in s.lines() {
            if let Some((p, d)) = line.split_once('\t') {
                if let Some(ms) = d.parse::<f64>().ok().filter(|ms| ms.is_finite()) {
                    m.insert(p.to_string(), ms);
                }
            }
        }
    }
    m
}

/// Persist the duration history (best effort: errors are ignored).
pub fn save_durations(path: &Path, m: &HashMap<String, f64>) {
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut out = String::new();
    for (p, d) in m {
        out.push_str(&format!("{p}\t{d}\n"));
    }
    let _ = std::fs::write(path, out);
}

/// Duration-aware order: slowest historical first (unknown = 0 ms; ties keep argv order —
/// work-stealing handles the rest). Returns indices into `files`. `total_cmp`, not
/// `partial_cmp().unwrap()`: a non-finite duration (which `load_durations` already drops) must
/// never panic the run.
pub fn schedule_order(files: &[PathBuf], hist: &HashMap<String, f64>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|&a, &b| {
        let da = hist.get(&history_key(&files[a])).copied().unwrap_or(0.0);
        let db = hist.get(&history_key(&files[b])).copied().unwrap_or(0.0);
        db.total_cmp(&da)
    });
    order
}

/// A file's absolute path: its duration-history key and its JSON report name (the launcher names
/// files relative to the working directory; the history is shared by runs from any directory).
/// Only an empty path has none: it is empty.
fn history_key(f: &Path) -> String {
    std::path::absolute(f)
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// The history to persist after a run: the old one with every file that ran updated.
pub fn update_history(
    mut hist: HashMap<String, f64>,
    files: &[PathBuf],
    res: &[FileResult],
) -> HashMap<String, f64> {
    for r in res {
        hist.insert(history_key(&files[r.idx]), r.dur_ms);
    }
    hist
}

/// The test-execution hooks [`execute_file`] / [`run_files`] drive. [`Engine::v8`] is the real
/// runner; tests substitute scripted closures.
pub struct Engine<'a> {
    /// Run one file (on the worker's reused isolate when reuse is on).
    pub run: &'a (dyn Fn(&Path) -> FileOutcome + Sync),
    /// Run one file on a FRESH isolate (panic-safe; restores the worker's reuse registry).
    pub run_fresh: &'a (dyn Fn(&Path) -> FileOutcome + Sync),
    /// Whether isolate reuse is active for this run.
    pub reuse_enabled: &'a (dyn Fn() -> bool + Sync),
    /// Panic recovery: leak the dead isolate's registry handles.
    pub forget_registry: &'a (dyn Fn() + Sync),
    /// End-of-worker teardown of the persistent isolate (no-op without reuse).
    pub end_worker: &'a (dyn Fn() + Sync),
}

impl Engine<'static> {
    /// The real V8 runner.
    pub fn v8() -> Self {
        Engine {
            run: &runner::run_test_file,
            run_fresh: &runner::run_test_file_fresh,
            reuse_enabled: &runner::is_reuse_enabled,
            forget_registry: &runner::forget_registry,
            end_worker: &runner::end_worker_reuse,
        }
    }
}

/// A caught panic payload as text (`&str` / `String` payloads; anything else → `"panic"`).
pub fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

/// The shared turbo-dom .node addon stores a process-global env that a concurrent worker's DOM
/// bootstrap can transiently clobber → "document is not defined" for a whole file. A whole file
/// failing (0 passed, ≥1 failed), an explicit DOM-not-ready error, or a load error is the
/// init-race signature worth re-running.
pub fn looks_transient(r: &FileOutcome) -> bool {
    match r {
        Ok(rep) => {
            (rep.passed == 0 && rep.failed > 0)
                || rep.failures.iter().any(|f| {
                    f.contains("is not defined")
                        && (f.contains("document") || f.contains("window") || f.contains("DOM"))
                })
        }
        Err(_) => true,
    }
}

/// Rank attempts: more passes first, then fewer failures; a load error is the worst.
pub fn score(r: &FileOutcome) -> i64 {
    match r {
        Ok(rep) => (rep.passed as i64) * 1000 - (rep.failed as i64),
        Err(_) => i64::MIN,
    }
}

/// Whether the outcome warrants the fresh-isolate retry (any failure, nothing passed, or error).
pub fn has_fail(r: &FileOutcome) -> bool {
    match r {
        Ok(rep) => rep.failed > 0 || (rep.passed == 0),
        Err(_) => true,
    }
}

/// Run one file: panic-guarded, re-run up to 2 more times while the result looks transient
/// (keeping the best attempt), then — under isolate reuse, when it still fails — re-run on a fresh
/// isolate whose (best-of-3) result is authoritative.
pub fn execute_file(engine: &Engine, file: &Path) -> FileOutcome {
    // A panic in V8/loader must not abort the whole run: catch it, leak the dead isolate's
    // handles, and report this file as an error.
    let run_once =
        || match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (engine.run)(file))) {
            Ok(r) => r,
            Err(p) => {
                (engine.forget_registry)();
                Err(format!("panicked: {}", panic_message(&*p)))
            }
        };
    // Re-run (up to 3x) and KEEP THE BEST attempt — a flaky file lands a clean pass on some
    // attempt; a genuinely all-failing file stays the same (just slower). This keeps the suite
    // deterministic across runs.
    let mut r = run_once();
    let mut tries = 0;
    while looks_transient(&r) && tries < 2 {
        tries += 1;
        let r2 = run_once();
        if score(&r2) > score(&r) {
            r = r2;
        }
    }
    // Fresh-isolate retry (idea #1): under reuse, a file that still has failures may be a
    // cross-file leak ARTIFACT (it passes in a clean isolate). Re-run it on a fresh isolate — that
    // result is authoritative (fresh mode is 6189/0). This pins reuse correctness to fresh while
    // keeping reuse speed for the files that pass.
    if (engine.reuse_enabled)() && has_fail(&r) {
        let mut fr = (engine.run_fresh)(file);
        let mut ft = 0;
        while looks_transient(&fr) && ft < 2 {
            ft += 1;
            let f2 = (engine.run_fresh)(file);
            if score(&f2) > score(&fr) {
                fr = f2;
            }
        }
        // fresh is the source of truth — adopt it (even if it also fails: that's real).
        r = fr;
    }
    r
}

/// How the runtime words vitest's file-level error for a file that collected no test or suite at
/// all (runtime.js `__tt.run`): the report's only `failures` entry, with no failed test.
pub const NO_SUITE_FOUND: &str = "No test suite found in file ";

/// Turn one file's outcome into the reporters' [`FileResult`]. A "no test suite found" report is
/// a file-level error, reported exactly like a load error (ERROR line, JSON `error`, JUnit
/// `<error>` case, TAP `not ok`, counted as a failed file).
pub fn file_result(idx: usize, file: &Path, r: FileOutcome, dur_ms: f64) -> FileResult {
    let r = match r {
        Ok(rep)
            if rep.failed == 0 && rep.failures.iter().any(|f| f.starts_with(NO_SUITE_FOUND)) =>
        {
            Err(rep.failures.join("; "))
        }
        r => r,
    };
    match r {
        Ok(rep) => {
            let mark = if rep.ok() { "PASS" } else { "FAIL" };
            let mut line = format!(
                "{mark}  {}  ({} passed, {} failed)",
                file.display(),
                rep.passed,
                rep.failed
            );
            for f in &rep.failures {
                line.push_str(&format!("\n        ✗ {f}"));
            }
            FileResult {
                idx,
                line,
                passed: rep.passed,
                failed: rep.failed,
                load_error: false,
                failed_file: !rep.ok(),
                error_msg: String::new(),
                tests: rep.tests,
                suites: rep.suites,
                snapshot: rep.snapshot,
                setup_us: rep.setup_us,
                dur_ms,
            }
        }
        Err(e) => FileResult {
            idx,
            line: format!("ERROR {}  ({e})", file.display()),
            passed: 0,
            failed: 0,
            load_error: true,
            failed_file: true,
            error_msg: e,
            tests: Vec::new(),
            suites: Vec::new(),
            snapshot: None,
            setup_us: 0.0,
            dur_ms,
        },
    }
}

/// The worker pool: `jobs` threads pull files in `order` from a shared cursor. Under `bail > 0`,
/// workers stop pulling NEW files once the cumulative failed-test count reaches `bail` (a worker
/// mid-file finishes that file — file-granular, see vitest.compat.md). Results sorted by index.
/// `names[i]` is how the reports name `files[i]` ([`display_names`]).
pub fn run_files(
    engine: &Engine,
    files: &[PathBuf],
    names: &[PathBuf],
    order: &[usize],
    jobs: usize,
    bail: usize,
) -> Vec<FileResult> {
    let cursor = AtomicUsize::new(0);
    let results = Mutex::new(Vec::<FileResult>::new());
    let failed_total = AtomicUsize::new(0);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..jobs)
            .map(|_| {
                s.spawn(|| {
                    loop {
                        if bail > 0 && failed_total.load(Ordering::Relaxed) >= bail {
                            break;
                        }
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= order.len() {
                            break;
                        }
                        let idx = order[i];
                        let file = &files[idx];
                        let t = Instant::now();
                        let r = execute_file(engine, file);
                        let fr =
                            file_result(idx, &names[idx], r, t.elapsed().as_secs_f64() * 1000.0);
                        if bail > 0 {
                            failed_total.fetch_add(fr.failed as usize, Ordering::Relaxed);
                        }
                        results.lock().unwrap().push(fr);
                    }
                    // reuse path: tear down this worker's persistent isolate cleanly (no-op otherwise)
                    (engine.end_worker)();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    });
    let mut res = results.into_inner().unwrap();
    res.sort_by_key(|r| r.idx);
    res
}

/// Run-wide counts.
#[derive(Debug, Default, PartialEq)]
pub struct Totals {
    pub passed: u32,
    pub failed: u32,
    /// Files that failed to load.
    pub errors: u32,
    /// Files that failed (to load, a test, or a suite / file-level error).
    pub failed_files: u32,
    /// Sum / count of env-setup µs over the files that loaded.
    pub setup_sum: f64,
    pub setup_n: u32,
}

pub fn tally(res: &[FileResult]) -> Totals {
    let mut t = Totals::default();
    for r in res {
        t.passed += r.passed;
        t.failed += r.failed;
        t.failed_files += u32::from(r.failed_file);
        if r.load_error {
            t.errors += 1;
        } else {
            t.setup_sum += r.setup_us;
            t.setup_n += 1;
        }
    }
    t
}

/// The human reporters' per-file stdout (default: PASS/FAIL lines; dot: one char per file + a
/// newline; verbose: PASS/FAIL line + one `✓/✗/- name (Nms)` line per test). Empty for artifacts.
pub fn render_human(reporter: Reporter, res: &[FileResult]) -> String {
    let mut out = String::new();
    for r in res {
        match reporter {
            Reporter::Default => {
                out.push_str(&r.line);
                out.push('\n');
            }
            Reporter::Dot => out.push_str(if r.load_error {
                "!"
            } else if !r.failed_file {
                "."
            } else {
                "x"
            }),
            Reporter::Verbose => {
                out.push_str(&r.line);
                out.push('\n');
                for t in &r.tests {
                    let mark = match t.status.as_str() {
                        "passed" => "✓",
                        "failed" => "✗",
                        _ => "-",
                    };
                    out.push_str(&format!("   {mark} {} ({:.0}ms)\n", t.name, t.duration_ms));
                }
            }
            _ => {}
        }
    }
    if reporter == Reporter::Dot {
        out.push('\n');
    }
    out
}

/// XML-escape a string for JUnit attribute/text content.
pub fn xml_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

/// A number as JavaScript's `JSON.stringify` writes it (integers without a fraction).
fn js_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e21 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// A JSON array of strings.
fn json_strs(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|s| json_str(s)).collect();
    format!("[{}]", items.join(","))
}

/// A load error's message as vitest's JSON `message` shows it: the error's own message, without
/// turbo-test's load-phase prefix (`module errored: `, …) and the error's `<Name>: ` prefix.
fn file_message(error_msg: &str) -> String {
    let mut m = error_msg;
    for p in ["module errored: ", "evaluate threw: ", "instantiate: "] {
        if let Some(rest) = m.strip_prefix(p) {
            m = rest;
            break;
        }
    }
    if let Some((name, rest)) = m.split_once(": ") {
        if name.ends_with("Error") && name.chars().all(|c| c.is_ascii_alphanumeric()) {
            m = rest;
        }
    }
    m.to_string()
}

/// vitest's `snapshot` summary block: the files' snapshot results added up as @vitest/snapshot's
/// `addSnapshotResult` does; `didUpdate` is whether `-u` / `--update` was given.
fn snapshot_summary(files: &[PathBuf], res: &[FileResult], did_update: bool) -> String {
    let (mut added, mut matched, mut unmatched, mut updated, mut unchecked) = (0, 0, 0, 0, 0);
    let (mut f_added, mut f_removed, mut f_unmatched, mut f_updated) = (0, 0, 0, 0);
    let mut by_file = Vec::new();
    for r in res {
        let Some(s) = &r.snapshot else { continue };
        f_added += u32::from(s.added > 0);
        f_removed += u32::from(s.file_deleted);
        f_unmatched += u32::from(s.unmatched > 0);
        f_updated += u32::from(s.updated > 0);
        added += s.added;
        matched += s.matched;
        unmatched += s.unmatched;
        updated += s.updated;
        unchecked += s.unchecked;
        if !s.unchecked_keys.is_empty() {
            by_file.push(format!(
                "{{\"filePath\":{},\"keys\":{}}}",
                json_str(&history_key(&files[r.idx])),
                json_strs(&s.unchecked_keys)
            ));
        }
    }
    let total = added + matched + unmatched + updated;
    format!(
        "{{\"added\":{added},\"failure\":false,\"filesAdded\":{f_added},\"filesRemoved\":{f_removed},\
         \"filesRemovedList\":[],\"filesUnmatched\":{f_unmatched},\"filesUpdated\":{f_updated},\
         \"matched\":{matched},\"total\":{total},\"unchecked\":{unchecked},\
         \"uncheckedKeysByFile\":[{}],\"unmatched\":{unmatched},\"updated\":{updated},\
         \"didUpdate\":{did_update}}}",
        by_file.join(",")
    )
}

/// One `assertionResults` entry, in vitest 4.1.12's key order (`duration` only for a test that
/// ran, `location` only under `includeTaskLocation`).
fn assertion_result(t: &TestCase) -> String {
    let mut full = t.ancestors.clone();
    if !t.title.is_empty() {
        full.push(t.title.clone());
    }
    let ran = (t.status == "passed" || t.status == "failed") && !t.not_run;
    let location = t.location.map_or(String::new(), |(line, column)| {
        format!(",\"location\":{{\"line\":{line},\"column\":{column}}}")
    });
    let duration = if ran {
        format!("\"duration\":{},", js_num(t.duration_ms))
    } else {
        String::new()
    };
    format!(
        "{{\"ancestorTitles\":{},\"fullName\":{},\"status\":{},\"title\":{},{duration}\"failureMessages\":{}{location},\"meta\":{{}},\"tags\":[]}}",
        json_strs(&t.ancestors),
        json_str(&full.join(" ")),
        json_str(&t.status),
        json_str(&t.title),
        json_strs(&t.failure_messages),
    )
}

/// `--reporter json`: vitest 4.1.12's JSON report — run-wide test-suite and test counts (the
/// files and their `describe` blocks are the suites), the snapshot summary, `startTime`
/// (`start_ms`, the run's start in epoch ms), `success`, and per file its `assertionResults`
/// (one per test, skipped / todo ones included), start / end time, status, load-error message
/// and absolute `name`. `did_update`: `-u` was given.
pub fn render_json(
    files: &[PathBuf],
    res: &[FileResult],
    start_ms: f64,
    did_update: bool,
) -> String {
    let count = |st: &str| -> usize {
        res.iter()
            .map(|r| r.tests.iter().filter(|t| t.status == st).count())
            .sum()
    };
    let total_tests: usize = res.iter().map(|r| r.tests.len()).sum();
    let (failed_tests, passed_tests) = (count("failed"), count("passed"));
    let suites = |st: &str| -> usize {
        res.iter()
            .map(|r| r.suites.iter().filter(|s| *s == st).count())
            .sum()
    };
    let total_suites = res.len() + res.iter().map(|r| r.suites.len()).sum::<usize>();
    let failed_suites = res.iter().filter(|r| r.failed_file).count() + suites("fail");
    let pending_suites = suites("todo");
    let mut out = String::from("{");
    out.push_str(&format!("\"numTotalTestSuites\":{total_suites},"));
    out.push_str(&format!(
        "\"numPassedTestSuites\":{},",
        total_suites - failed_suites - pending_suites
    ));
    out.push_str(&format!("\"numFailedTestSuites\":{failed_suites},"));
    out.push_str(&format!("\"numPendingTestSuites\":{pending_suites},"));
    out.push_str(&format!("\"numTotalTests\":{total_tests},"));
    out.push_str(&format!("\"numPassedTests\":{passed_tests},"));
    out.push_str(&format!("\"numFailedTests\":{failed_tests},"));
    out.push_str(&format!("\"numPendingTests\":{},", count("skipped")));
    out.push_str(&format!("\"numTodoTests\":{},", count("todo")));
    out.push_str(&format!(
        "\"snapshot\":{},",
        snapshot_summary(files, res, did_update)
    ));
    out.push_str(&format!("\"startTime\":{},", js_num(start_ms)));
    out.push_str(&format!(
        "\"success\":{},",
        !files.is_empty() && failed_suites == 0 && failed_tests == 0
    ));
    out.push_str("\"testResults\":[");
    let items: Vec<String> = res
        .iter()
        .map(|r| {
            let ran: Vec<&TestCase> = r
                .tests
                .iter()
                .filter(|t| t.status == "passed" || t.status == "failed")
                .collect();
            let start = ran
                .iter()
                .map(|t| t.start_ms)
                .reduce(f64::min)
                .unwrap_or(start_ms);
            let end = ran
                .iter()
                .map(|t| t.start_ms + t.duration_ms)
                .fold(start, f64::max);
            let results: Vec<String> = r.tests.iter().map(assertion_result).collect();
            format!(
                "{{\"assertionResults\":[{}],\"startTime\":{},\"endTime\":{},\"status\":\"{}\",\"message\":{},\"name\":{}}}",
                results.join(","),
                js_num(start),
                js_num(end),
                if r.failed_file { "failed" } else { "passed" },
                json_str(&if r.load_error {
                    file_message(&r.error_msg)
                } else {
                    String::new()
                }),
                json_str(&history_key(&files[r.idx])),
            )
        })
        .collect();
    out.push_str(&items.join(","));
    out.push_str("]}");
    out
}

/// JUnit XML: one `<testsuite>` per file, one `<testcase>` per non-skipped test.
pub fn render_junit(files: &[PathBuf], res: &[FileResult], t: &Totals) -> String {
    let total: u32 = res.iter().map(|r| r.passed + r.failed).sum();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"turbo-test\" tests=\"{total}\" failures=\"{}\" errors=\"{}\">\n",
        t.failed, t.errors
    ));
    for r in res {
        let suite = files[r.idx].to_string_lossy().to_string();
        let secs = r.dur_ms / 1000.0;
        // A file that failed to LOAD has no per-test list: emit one synthetic failing
        // <testcase> carrying the load error, so the failure is never silently dropped.
        if r.load_error {
            out.push_str(&format!(
                "  <testsuite name=\"{}\" tests=\"1\" failures=\"1\" errors=\"1\" time=\"{:.3}\">\n",
                xml_esc(&suite),
                secs
            ));
            out.push_str(&format!(
                "    <testcase name=\"{}\" classname=\"{}\">\n      <error message=\"{}\"/>\n    </testcase>\n",
                xml_esc(&suite),
                xml_esc(&suite),
                xml_esc(&r.error_msg)
            ));
            out.push_str("  </testsuite>\n");
            continue;
        }
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" time=\"{:.3}\">\n",
            xml_esc(&suite),
            r.passed + r.failed,
            r.failed,
            secs
        ));
        for tc in &r.tests {
            if tc.status == "skipped" {
                continue;
            }
            out.push_str(&format!(
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\"",
                xml_esc(&tc.name),
                xml_esc(&suite),
                tc.duration_ms / 1000.0
            ));
            if tc.status == "failed" {
                out.push_str(&format!(
                    ">\n      <failure message=\"{}\"/>\n    </testcase>\n",
                    xml_esc(&tc.message)
                ));
            } else {
                out.push_str("/>\n");
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// TAP v13: a flat numbered plan over every test across all files. A file that failed to LOAD has
/// no per-test list: it is one `not ok N - <file>` point carrying the load error (the TAP twin of
/// JUnit's synthetic `<error>` case), so the failure is never silently dropped from the plan.
pub fn render_tap(files: &[PathBuf], res: &[FileResult]) -> String {
    let total: usize = res
        .iter()
        .map(|r| if r.load_error { 1 } else { r.tests.len() })
        .sum();
    let mut out = String::from("TAP version 13\n");
    out.push_str(&format!("1..{total}\n"));
    let diag = |out: &mut String, message: &str| {
        out.push_str("  ---\n");
        out.push_str(&format!("  message: {}\n", message.replace('\n', " ")));
        out.push_str("  ...\n");
    };
    let mut n = 0usize;
    for r in res {
        if r.load_error {
            n += 1;
            out.push_str(&format!("not ok {n} - {}\n", files[r.idx].display()));
            diag(&mut out, &r.error_msg);
            continue;
        }
        for t in &r.tests {
            n += 1;
            let ok = if t.status == "failed" { "not ok" } else { "ok" };
            let dir = if t.status == "skipped" { " # SKIP" } else { "" };
            out.push_str(&format!("{ok} {n} - {}{dir}\n", t.name));
            if t.status == "failed" && !t.message.is_empty() {
                diag(&mut out, &t.message);
            }
        }
    }
    out
}

/// The active artifact reporter's output (json/junit/tap); `None` for the human reporters.
/// JUnit / TAP name the files by `names`, the JSON report by their absolute `files` path;
/// `start_ms` (the run's start, epoch ms) and `did_update` (`-u`) feed the JSON report.
pub fn render_artifact(
    reporter: Reporter,
    files: &[PathBuf],
    names: &[PathBuf],
    res: &[FileResult],
    t: &Totals,
    start_ms: f64,
    did_update: bool,
) -> Option<String> {
    match reporter {
        Reporter::Json => Some(render_json(files, res, start_ms, did_update)),
        Reporter::Junit => Some(render_junit(names, res, t)),
        Reporter::Tap => Some(render_tap(names, res)),
        _ => None,
    }
}

/// Write `text` to the `--outputFile` `path`, creating missing parent dirs. Returns the stderr
/// message for a failed write (empty on success).
fn write_output_file(path: &str, text: &str) -> String {
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, text) {
        Ok(()) => String::new(),
        Err(e) => format!("turbo-test: failed to write --outputFile '{path}': {e}\n"),
    }
}

/// Deliver the artifact: to `output_file` when given (parent dirs created), else to stdout. With
/// a human reporter + `--outputFile`, the plain PASS/FAIL lines are written there (parent dirs
/// created too) so the flag is never silently ignored. Returns the `(stdout, stderr)` text to
/// print; stderr is non-empty exactly when writing `--outputFile` failed, which fails the run.
pub fn emit_output(
    artifact: Option<&str>,
    reporter: Reporter,
    output_file: Option<&str>,
    res: &[FileResult],
) -> (String, String) {
    let (mut stdout, mut stderr) = (String::new(), String::new());
    if let Some(text) = artifact {
        match output_file {
            Some(path) => stderr = write_output_file(path, text),
            None => stdout = format!("{text}\n"),
        }
    } else if let (Some(path), false) = (output_file, reporter.is_artifact()) {
        let text: String = res.iter().map(|r| format!("{}\n", r.line)).collect();
        stderr = write_output_file(path, &text);
    }
    (stdout, stderr)
}

/// Wall time as vitest prints it: `1.23s` from one second up, else `456ms`.
pub fn format_duration(wall_ms: f64) -> String {
    if wall_ms >= 1000.0 {
        format!("{:.2}s", wall_ms / 1000.0)
    } else {
        format!("{:.0}ms", wall_ms)
    }
}

/// `a failed | b passed (total)`, or just `b passed (total)` when nothing failed.
pub fn count_line(failed: usize, passed: usize, total: usize) -> String {
    if failed > 0 {
        format!("{failed} failed | {passed} passed ({total})")
    } else {
        format!("{passed} passed ({total})")
    }
}

/// Vitest-style summary footer (`Test Files` / `Tests` / `Duration`). A file counts as failed if
/// any test failed OR it failed to load. Counted from actual results, not `file_count` — under
/// `--bail` some files never run, and those must NOT be reported as passed; the total stays the
/// discovered file count.
pub fn footer(file_count: usize, res: &[FileResult], t: &Totals, wall_ms: f64) -> String {
    let failed_files = res.iter().filter(|r| r.failed_file).count();
    let passed_files = res.len() - failed_files;
    format!(
        "\n Test Files  {}\n      Tests  {}\n   Duration  {}",
        count_line(failed_files, passed_files, file_count),
        count_line(
            t.failed as usize,
            t.passed as usize,
            (t.passed + t.failed) as usize
        ),
        format_duration(wall_ms),
    )
}

/// turbo-test's diagnostic summary line (files, counts, jobs, wall, env setup, cache hit-rate).
pub fn summary(
    file_count: usize,
    t: &Totals,
    jobs: usize,
    wall_ms: f64,
    cache: (u64, u64),
) -> String {
    let avg_setup = if t.setup_n > 0 {
        t.setup_sum / t.setup_n as f64
    } else {
        0.0
    };
    let (hits, misses) = cache;
    let hit_rate = if hits + misses > 0 {
        100.0 * hits as f64 / (hits + misses) as f64
    } else {
        0.0
    };
    format!(
        "\n{} files | {} passed | {} failed | {} load-errors | {} jobs | wall {:.0} ms | env setup {:.2} ms/file | cache {:.0}% hit",
        file_count,
        t.passed,
        t.failed,
        t.errors,
        jobs,
        wall_ms,
        avg_setup / 1000.0,
        hit_rate
    )
}

/// 0 when nothing failed (no failed test, no file that failed to load or reported a suite /
/// file-level error) and the run's other gates passed (`gates_ok`: the coverage gate, if any, and
/// the `--outputFile` write); else 1.
pub fn exit_code(t: &Totals, gates_ok: bool) -> i32 {
    if t.failed == 0 && t.errors == 0 && t.failed_files == 0 && gates_ok {
        0
    } else {
        1
    }
}

/// The whole CLI: launcher front-end (subcommand stripping, config/discovery, `--changed`, …;
/// may exit for terminal cases), then [`parse_args`] + [`run`]. Returns the exit code.
pub fn main_with_args(raw: Vec<String>) -> i32 {
    let jobs = default_jobs(std::env::var("TURBO_JOBS").ok().as_deref());
    run(parse_args(crate::launcher::prepare(raw), jobs))
}

/// Apply `opts` (env, coverage config), run every file and print the reports. Returns the exit
/// code (2 = usage: no test files).
pub fn run(opts: Options) -> i32 {
    for w in &opts.warnings {
        eprintln!("{w}");
    }
    for (k, v) in &opts.env {
        std::env::set_var(k, v);
    }
    for op in &opts.coverage {
        op.apply();
    }
    if opts.files.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }
    // allowOnly off (--no-allowOnly, or vitest's default on CI): plumb to the runtime, which
    // fails the `.only` tasks.
    if !opts.allow_only.unwrap_or_else(|| !crate::ci::is_ci_env()) {
        std::env::set_var("TURBO_TEST_FORBID_ONLY", "1");
    }
    let files = apply_shard(opts.files, opts.shard);
    // An artifact printed to stdout must be the ONLY thing there: test console output goes to
    // stderr then (like the summary line below).
    let artifact_on_stdout = opts.reporter.is_artifact() && opts.output_file.is_none();
    runner::CONSOLE_TO_STDERR.store(artifact_on_stdout, Ordering::Relaxed);

    runner::init_v8(); // builds the framework snapshot once, before workers spawn

    let hist_path = durations_path();
    let hist = load_durations(&hist_path);
    let order = schedule_order(&files, &hist);
    let jobs = opts.jobs.min(files.len()).max(1);

    let start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64);
    let wall = Instant::now();
    let names = display_names(
        &files,
        std::env::var_os("TURBO_ROOT").map(PathBuf::from).as_deref(),
    );
    let res = run_files(&Engine::v8(), &files, &names, &order, jobs, opts.bail);
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;

    let t = tally(&res);
    print!("{}", render_human(opts.reporter, &res));
    save_durations(&hist_path, &update_history(hist, &files, &res));

    let did_update = opts.env.iter().any(|(k, _)| *k == "TURBO_UPDATE_SNAPSHOTS");
    let artifact = render_artifact(
        opts.reporter,
        &files,
        &names,
        &res,
        &t,
        start_ms,
        did_update,
    );
    let (out, err) = emit_output(
        artifact.as_deref(),
        opts.reporter,
        opts.output_file.as_deref(),
        &res,
    );
    print!("{out}");
    eprint!("{err}");
    let output_ok = err.is_empty();

    let summary = summary(
        files.len(),
        &t,
        jobs,
        wall_ms,
        runner::transform_cache_stats(),
    );
    // Keep stdout clean for machine artifacts (json/junit/tap) printed to stdout — send the human
    // summary to stderr so it doesn't corrupt the parsed output.
    if artifact_on_stdout {
        eprintln!("{summary}");
    } else {
        // Human reporters get the vitest-style footer; the turbo diagnostic line follows it.
        println!("{}", footer(files.len(), &res, &t, wall_ms));
        println!("{summary}");
    }
    let cov_ok = if coverage::enabled() {
        coverage::report()
    } else {
        true
    };
    exit_code(&t, cov_ok && output_ok)
}

#[cfg(all(test, unix))]
#[path = "cli_tests.rs"]
mod tests;
