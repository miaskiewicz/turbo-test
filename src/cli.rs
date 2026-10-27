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
    /// The suites whose hooks failed (junit test cases of their own).
    pub suite_errors: Vec<runner::SuiteError>,
    /// The error a file failed to load with, when the runner saw one (junit's `<failure>`).
    pub load_error_info: Option<crate::failure::ErrorInfo>,
    /// Its console output: (`stdout` / `stderr`, the test's or suite's name path or `""` for
    /// the file, content), for vitest's human reporters.
    pub logs: Vec<(String, String, String)>,
    /// ms its tests waited on timers (the drive loop's clock jumps, turbo-test#95): vitest's
    /// file duration counts them.
    pub waited_ms: f64,
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
    /// `--reporter` was given (without it, vitest picks its agent reporter under an AI agent).
    pub reporter_explicit: bool,
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
        reporter_explicit: false,
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
            "--reporter" | "--reporters" => {
                o.reporter = Reporter::parse(value().as_deref());
                o.reporter_explicit = true;
            }
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

/// Whether a file is worth re-running on the same kind of isolate: only a file that did not run
/// its tests (a load error, a panic). A report — however many of its tests failed — is what
/// vitest reports from its one run of the file (turbo-test#144); the old all-failing /
/// "document is not defined" heuristic guarded a DOM init race in the shared turbo-dom `.node`
/// addon, which the in-process turbo-dom-browser-env binding does not have.
pub fn looks_transient(r: &FileOutcome) -> bool {
    match r {
        Ok(_) => false,
        // A file the heap guard stopped: same-mode retries would only run out of memory again
        // (under reuse the fresh-isolate retry below still gives it a clean heap).
        Err(e) => !crate::heap_guard::is_oom(e),
    }
}

/// Rank attempts: more passes first, then fewer failures; a load error is the worst.
pub fn score(r: &FileOutcome) -> i64 {
    match r {
        Ok(rep) => (rep.passed as i64) * 1000 - (rep.failed as i64),
        Err(_) => i64::MIN,
    }
}

/// Whether a file's run under isolate reuse is what vitest `isolate: false` reports
/// (turbo-test#126): its tests were collected (no load error, no "no test suite" report, no
/// failed hook) and each failed test failed an assertion. vitest runs each file once on the shared worker and
/// reports that run — a test failing an `expect` is reported, even when state an earlier file
/// left caused it. Anything else (a test throwing a TypeError, a load error, a panic, the heap
/// guard's out-of-memory abort) may be the reused isolate's own state — a turbo-test reuse
/// artifact vitest would not show — and gets one run on a fresh isolate.
pub fn reuse_result_is_final(r: &FileOutcome) -> bool {
    let Ok(rep) = r else { return false };
    let failed: Vec<&TestCase> = rep.tests.iter().filter(|t| t.status == "failed").collect();
    rep.suite_errors.is_empty()
        && !rep.failures.iter().any(|f| f.starts_with(NO_SUITE_FOUND))
        && !rep.tests.is_empty()
        && failed.len() == rep.failed as usize
        && failed
            .iter()
            .all(|t| !t.errors.is_empty() && t.errors.iter().all(|e| e.name == "AssertionError"))
}

/// Run one file: panic-guarded, re-run up to 2 more times while it did not run its tests
/// ([`looks_transient`], keeping the best attempt), then — under isolate reuse, unless the result is final
/// ([`reuse_result_is_final`]) — re-run on a fresh isolate whose (best-of-3) result is
/// authoritative.
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
    // A file that did not run its tests is re-run (up to 3x), keeping the best attempt. A report
    // never is (turbo-test#144) — under reuse a second run on the reused isolate would also find
    // the file's module already evaluated and collect no tests ("No test suite found").
    let mut r = run_once();
    let mut tries = 0;
    let reuse = (engine.reuse_enabled)();
    while looks_transient(&r) && tries < 2 {
        tries += 1;
        let r2 = run_once();
        if score(&r2) > score(&r) {
            r = r2;
        }
    }
    // Fresh-isolate retry (turbo-test#126): reuse is vitest's `isolate: false` (the config or
    // `--no-isolate`), which runs each file once and reports it, so a passing file or one whose
    // failures are assertion failures is final — re-running it fresh doubled its cost and could
    // only hide what vitest shows. Any other result may be a reuse artifact (a module left
    // half-linked by an earlier file, a stale DOM wrapper, the accumulated heap): one run on a
    // fresh isolate, whose result is authoritative.
    if reuse && !reuse_result_is_final(&r) {
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
/// a file-level error, reported exactly like a load error (ERROR line, JSON `error`, a failing
/// JUnit case, TAP `not ok`, counted as a failed file).
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
                suite_errors: rep.suite_errors,
                load_error_info: None,
                logs: rep.logs,
                waited_ms: rep.waited_ms,
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
            suite_errors: Vec::new(),
            load_error_info: runner::take_load_error(),
            logs: Vec::new(),
            waited_ms: 0.0,
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

/// The legacy human reporters' (`TURBO_LEGACY_REPORTER=1`) per-file stdout (default: PASS/FAIL lines; dot: one char per file + a
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

/// XML-escape a string for JUnit attribute/text content, as vitest's `escapeXML`: the five
/// entities, and characters XML 1.0 forbids or discourages left out.
pub fn xml_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\0'..='\x08'
            | '\x0b'
            | '\x0c'
            | '\x0e'..='\x1f'
            | '\u{7f}'..='\u{84}'
            | '\u{86}'..='\u{9f}' => {}
            '\u{fdd0}'..='\u{fdef}' | '\u{fffd}'..='\u{ffff}' => {}
            c if (c as u32) & 0xfffe == 0xfffe => {}
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
pub(crate) fn file_message(error_msg: &str) -> String {
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
        {
            let items: Vec<String> = t
                .failure_messages
                .iter()
                .map(|m| m.as_deref().map_or("null".to_string(), json_str))
                .collect();
            format!("[{}]", items.join(","))
        },
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

/// JUnit XML as vitest 4.1.12's junit reporter writes it (turbo-test#81): `<testsuites name="vitest
/// tests">`, a `<testsuite>` per file (`timestamp`, `hostname`, `skipped`), a `<testcase>` per
/// test (skipped / todo ones with `<skipped/>`) with its console output (`<system-out>` /
/// `<system-err>`) and its `<failure message type>` holding the printed error
/// ([`crate::failure::print_error`]), then a test case per suite whose hooks failed; a file that
/// failed to load is one failing test case named after the file. Four-space indentation, every
/// element's closing tag on a line of its own; durations in seconds as JavaScript prints them.
/// `root` is the directory the failure frames are named relative to.
pub fn render_junit(names: &[PathBuf], res: &[FileResult], root: &Path) -> String {
    let host = hostname();
    let mut lines: Vec<String> = vec!["<?xml version=\"1.0\" encoding=\"UTF-8\" ?>".to_string()];
    let mut suites: Vec<Vec<String>> = Vec::new();
    let (mut tests, mut failures, mut time) = (0usize, 0usize, 0.0f64);
    for r in res {
        let name = names[r.idx].to_string_lossy().to_string();
        let mut cases: Vec<String> = Vec::new();
        let (mut n, mut failed, mut skipped) = (0usize, 0usize, 0usize);
        for t in &r.tests {
            n += 1;
            let mut body = Vec::new();
            for (stream, tag) in [("stdout", "system-out"), ("stderr", "system-err")] {
                let logs: Vec<&(String, String)> = r_logs(t, stream);
                if !logs.is_empty() {
                    body.push(format!("            <{tag}>"));
                    for (_, content) in logs {
                        body.push(xml_esc(content));
                    }
                    body.push(format!("            </{tag}>"));
                }
            }
            if t.status == "skipped" || t.status == "todo" {
                skipped += 1;
                body.push("            <skipped/>".to_string());
            }
            // its annotations, one property each (turbo-test#96)
            if !t.annotations.is_empty() {
                body.push("            <properties>".to_string());
                for a in &t.annotations {
                    body.push(format!(
                        "                <property name=\"{}\" value=\"{}\">",
                        xml_esc(&a.kind),
                        xml_esc(&a.message)
                    ));
                    body.push("                </property>".to_string());
                }
                body.push("            </properties>".to_string());
            }
            if t.status == "failed" {
                failed += 1;
                // a failure the runtime recorded no error object for (a forbidden `.only`)
                let fallback = [load_error_info(&t.message)];
                let errors = if t.errors.is_empty() {
                    &fallback[..]
                } else {
                    &t.errors[..]
                };
                for e in errors {
                    body.extend(failure_element(e, root));
                }
            }
            cases.push(testcase(&name, &t.name, t.duration_ms, body));
        }
        for se in &r.suite_errors {
            n += 1;
            failed += 1;
            let body = se
                .errors
                .iter()
                .flat_map(|e| failure_element(e, root))
                .collect();
            cases.push(testcase(&name, &se.name, se.duration_ms, body));
        }
        if n == 0 && r.failed_file {
            n = 1;
            failed = 1;
            let e = r
                .load_error_info
                .clone()
                .unwrap_or_else(|| load_error_info(&r.error_msg));
            cases.push(testcase(&name, &name, 0.0, failure_element(&e, root)));
        }
        let dur = if r.load_error { 0.0 } else { r.dur_ms };
        let mut suite = vec![format!(
            "    <testsuite name=\"{}\" timestamp=\"{}\" hostname=\"{}\" tests=\"{n}\" failures=\"{failed}\" errors=\"0\" skipped=\"{skipped}\" time=\"{}\">",
            xml_esc(&name),
            iso_now(),
            xml_esc(&host),
            seconds(dur)
        )];
        suite.extend(cases);
        suite.push("    </testsuite>".to_string());
        suites.push(suite);
        tests += n;
        failures += failed;
        time += dur;
    }
    lines.push(format!(
        "<testsuites name=\"vitest tests\" tests=\"{tests}\" failures=\"{failures}\" errors=\"0\" time=\"{}\">",
        seconds(time)
    ));
    lines.extend(suites.into_iter().flatten());
    lines.push("</testsuites>".to_string());
    lines.join("\n")
}

/// A test's captured console output on `stream`.
fn r_logs<'t>(t: &'t TestCase, stream: &str) -> Vec<&'t (String, String)> {
    t.logs.iter().filter(|(s, _)| s == stream).collect()
}

/// One `<testcase>` with its child lines.
fn testcase(classname: &str, name: &str, ms: f64, body: Vec<String>) -> String {
    let mut out = vec![format!(
        "        <testcase classname=\"{}\" name=\"{}\" time=\"{}\">",
        xml_esc(classname),
        xml_esc(name),
        seconds(ms)
    )];
    out.extend(body);
    out.push("        </testcase>".to_string());
    out.join("\n")
}

/// A `<failure>` element's lines for `e`.
/// Its `message` / `type` attributes are the error's message and name, each left out when the
/// error has none (turbo-test#97: a thrown primitive has no name, an object maybe neither).
fn failure_element(e: &crate::failure::ErrorInfo, root: &Path) -> Vec<String> {
    let mut open = "            <failure".to_string();
    if !e.no_message {
        open.push_str(&format!(" message=\"{}\"", xml_esc(&e.message)));
    }
    if !e.no_name {
        open.push_str(&format!(" type=\"{}\"", xml_esc(&e.name)));
    }
    open.push('>');
    vec![
        open,
        xml_esc(&crate::failure::print_error(e, root)),
        "            </failure>".to_string(),
    ]
}

/// A load error without the runner's error object (a setup file, a graph that did not load):
/// its message as `Name: message` (the error name, else `Error`).
fn load_error_info(error_msg: &str) -> crate::failure::ErrorInfo {
    let mut m = error_msg;
    for p in ["module errored: ", "evaluate threw: ", "instantiate: "] {
        if let Some(rest) = m.strip_prefix(p) {
            m = rest;
            break;
        }
    }
    let (name, message) = match m.split_once(": ") {
        Some((name, rest))
            if name.ends_with("Error") && name.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (name, rest)
        }
        _ => ("Error", m),
    };
    crate::failure::ErrorInfo {
        name: name.to_string(),
        message: message.to_string(),
        ..Default::default()
    }
}

/// Milliseconds as vitest's junit `executionTime` prints seconds: `(ms / 1000).toLocaleString('en-US',
/// { useGrouping: false, maximumFractionDigits: 10 })`.
pub fn seconds(ms: f64) -> String {
    let s = format!("{:.10}", ms / 1000.0);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The current time as JavaScript's `new Date().toISOString()` (UTC, milliseconds).
fn iso_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    iso_time(now.as_millis() as i64)
}

/// `new Date(ms).toISOString()` for a time after 1970.
pub fn iso_time(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// The machine's host name, as Node's `os.hostname()` (the `hostname` command; `localhost`
/// when it cannot run).
fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

/// TAP v13: a flat numbered plan over every test across all files. A file that failed to LOAD has
/// no per-test list: it is one `not ok N - <file>` point carrying the load error (the TAP twin of
/// JUnit's failing case for the file), so the failure is never silently dropped from the plan.
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
    start_ms: f64,
    did_update: bool,
) -> Option<String> {
    match reporter {
        Reporter::Json => Some(render_json(files, res, start_ms, did_update)),
        Reporter::Junit => Some(render_junit(names, res, &junit_root())),
        Reporter::Tap => Some(render_tap(names, res)),
        _ => None,
    }
}

/// The vitest human reporter for this run (`None`: an artifact reporter, or
/// `TURBO_LEGACY_REPORTER` set to anything but `0` — turbo-test's own PASS / FAIL lines).
pub fn human_kind(reporter: Reporter, explicit: bool) -> Option<crate::human::Kind> {
    let legacy = std::env::var("TURBO_LEGACY_REPORTER").is_ok_and(|v| !v.is_empty() && v != "0");
    human_kind_for(reporter, explicit, legacy, crate::agent::is_agent_env())
}

/// [`human_kind`] for a `legacy` switch and whether an AI `agent` runs the tests: without
/// `--reporter` (`explicit`) an agent gets vitest's agent reporter.
pub fn human_kind_for(
    reporter: Reporter,
    explicit: bool,
    legacy: bool,
    agent: bool,
) -> Option<crate::human::Kind> {
    use crate::human::Kind;
    match reporter {
        _ if legacy => None,
        Reporter::Verbose => Some(Kind::Verbose),
        Reporter::Dot => Some(Kind::Dot),
        Reporter::Default if agent && !explicit => Some(Kind::Agent),
        Reporter::Default => Some(Kind::Default),
        _ => None,
    }
}

/// `new Date(ms).toTimeString().split(' ')[0]`: the local `HH:MM:SS` (`TZ` decides the zone).
pub fn local_hms(epoch_ms: f64) -> String {
    let secs = (epoch_ms / 1000.0) as libc::time_t;
    // SAFETY: localtime_r writes into `tm`, which outlives the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// The directory junit names failure frames relative to: `--root` (TURBO_ROOT), else the
/// working directory.
fn junit_root() -> PathBuf {
    let root = std::env::var_os("TURBO_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    std::fs::canonicalize(&root).unwrap_or(root)
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
            Some(path) => {
                // vitest's junit writes each line with its newline; its JSON report is one line
                // without one. Both say where the report went (`JUNIT report written to …`).
                let (kind, text) = match reporter {
                    Reporter::Junit => ("JUNIT", format!("{text}\n")),
                    Reporter::Json => ("JSON", text.to_string()),
                    _ => ("", text.to_string()),
                };
                stderr = write_output_file(path, &text);
                if stderr.is_empty() && !kind.is_empty() {
                    let abs = std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path));
                    stdout = format!("{kind} report written to {}\n", abs.display());
                }
            }
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

/// vitest's `getStateString`: `a failed | b passed | c expected fail | d skipped | e todo
/// (total)` from `[failed, passed, expected fail, skipped, todo]`, the zero parts left out, and
/// `no tests` when the total is 0 (turbo-test#78).
pub fn state_string(counts: [usize; 5], total: usize) -> String {
    state_string_colored(counts, total, &crate::colors::Colors::default())
}

/// [`state_string`] in vitest's colours: failed bold red, passed bold green, expected fail cyan,
/// skipped yellow, todo gray, the separators dim, the total gray (`no tests` dim).
pub fn state_string_colored(counts: [usize; 5], total: usize, c: &crate::colors::Colors) -> String {
    if total == 0 {
        return c.dim("no tests");
    }
    let parts: Vec<String> = counts
        .iter()
        .zip(["failed", "passed", "expected fail", "skipped", "todo"])
        .filter(|(n, _)| **n > 0)
        .map(|(n, label)| {
            let text = format!("{n} {label}");
            match label {
                "failed" => c.bold(&c.red(&text)),
                "passed" => c.bold(&c.green(&text)),
                "expected fail" => c.cyan(&text),
                "skipped" => c.yellow(&text),
                _ => c.gray(&text),
            }
        })
        .collect();
    format!(
        "{}{}",
        parts.join(&c.dim(" | ")),
        c.gray(&format!(" ({total})"))
    )
}

/// Vitest's summary footer (`Test Files` / `Tests` / `Duration`), both lines as its
/// `getStateString`. A file fails if it failed to load, a test failed or a suite / file-level
/// error was reported; one with tasks none of which ran (skipped / todo / pending tests, or
/// only skipped / todo suites) is `skipped`.
/// The `Tests` total counts every test record, pending ones included. Counted from actual
/// results, not `file_count` — under `--bail` some files never run, and those must NOT be
/// reported as passed; the total stays the discovered file count.
pub fn footer(file_count: usize, res: &[FileResult], wall_ms: f64) -> String {
    let (files, tests) = footer_counts(file_count, res, &crate::colors::Colors::default());
    format!(
        "\n Test Files  {files}\n      Tests  {tests}\n   Duration  {}",
        format_duration(wall_ms),
    )
}

/// The footer's `Test Files` and `Tests` values (vitest's `getStateString`).
pub fn footer_counts(
    file_count: usize,
    res: &[FileResult],
    c: &crate::colors::Colors,
) -> (String, String) {
    let mut files = [0usize; 5];
    for r in res {
        let ran = r
            .tests
            .iter()
            .any(|t| t.status == "passed" || t.status == "failed");
        let slot = if r.failed_file {
            0
        } else if !ran && !(r.tests.is_empty() && r.suites.is_empty()) {
            3
        } else {
            1
        };
        files[slot] += 1;
    }
    let mut tests = [0usize; 5];
    let all: Vec<&TestCase> = res.iter().flat_map(|r| &r.tests).collect();
    for t in &all {
        let slot = match t.status.as_str() {
            "failed" => 0,
            "passed" if t.expected_fail => 2,
            "passed" => 1,
            "skipped" => 3,
            "todo" => 4,
            _ => continue,
        };
        tests[slot] += 1;
    }
    (
        state_string_colored(files, file_count, c),
        state_string_colored(tests, all.len(), c),
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
    // vitest's human reporters (turbo-test#94) unless TURBO_LEGACY_REPORTER keeps turbo-test's
    // compact PASS / FAIL lines; without --reporter an AI agent gets vitest's agent reporter.
    // They, and junit (vitest), print each task's console output with its task, and failure
    // frames named in the original source (the test file's transform map, location.rs)
    let human = human_kind(opts.reporter, opts.reporter_explicit);
    // vitest colours its human reporters as tinyrainbow decides, never under an AI agent; the
    // runtime colours console values and diffs to match
    let colors = crate::colors::Colors {
        on: human.is_some()
            && !crate::agent::is_agent_env()
            && crate::colors::supported(
                |k| std::env::var(k).ok(),
                &std::env::args().collect::<Vec<_>>(),
            ),
    };
    if colors.on {
        std::env::set_var("TURBO_COLORS", "1");
    }
    if human.is_some() || opts.reporter == Reporter::Junit {
        std::env::set_var("TURBO_CAPTURE_LOGS", "1");
        std::env::set_var("TURBO_TASK_LOCATION", "1");
        std::env::set_var("TURBO_ERROR_FRAMES", "1");
    }

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
    if let Some(kind) = human {
        save_durations(&hist_path, &update_history(hist, &files, &res));
        let root = junit_root();
        let (test_files, tests) = footer_counts(files.len(), &res, &colors);
        let run = crate::human::Run {
            root: &root,
            names: &names,
            wall_ms,
            start: local_hms(start_ms),
            test_files,
            tests,
            did_update: opts.env.iter().any(|(k, _)| *k == "TURBO_UPDATE_SNAPSHOTS"),
            colors,
        };
        let (out, err) = crate::human::render(kind, &res, &run);
        // vitest's human reporters write no --outputFile
        print!("{out}");
        eprint!("{err}");
        let cov_ok = !coverage::enabled() || coverage::report();
        return exit_code(&t, cov_ok);
    }
    print!("{}", render_human(opts.reporter, &res));
    save_durations(&hist_path, &update_history(hist, &files, &res));

    let did_update = opts.env.iter().any(|(k, _)| *k == "TURBO_UPDATE_SNAPSHOTS");
    let artifact = render_artifact(opts.reporter, &files, &names, &res, start_ms, did_update);
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
    // summary to stderr so it doesn't corrupt the parsed output. Written to --outputFile, the
    // artifact reporters print no footer either (vitest prints only `… report written to …`).
    if opts.reporter.is_artifact() {
        eprintln!("{summary}");
    } else {
        // Human reporters get the vitest-style footer; the turbo diagnostic line follows it.
        println!("{}", footer(files.len(), &res, wall_ms));
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
