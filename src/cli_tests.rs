//! Unit tests for `cli.rs`: argument parsing, sharding, duration history + scheduling, the
//! panic guard / transient retry / fresh-isolate retry (against a scripted `Engine`), the worker
//! pool + `--bail`, and every reporter renderer.

use super::*;
use std::sync::atomic::AtomicUsize;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn parse(a: &[&str]) -> Options {
    parse_args(args(a), 7)
}

fn report(passed: u32, failed: u32) -> TestReport {
    TestReport {
        passed,
        failed,
        ..Default::default()
    }
}

fn case(name: &str, status: &str, ms: f64, message: &str) -> TestCase {
    let mut parts: Vec<String> = name.split(" > ").map(str::to_string).collect();
    let title = parts.pop().unwrap();
    TestCase {
        name: name.to_string(),
        title,
        ancestors: parts,
        status: status.to_string(),
        duration_ms: ms,
        start_ms: if status == "passed" || status == "failed" {
            1000.0
        } else {
            0.0
        },
        message: message.to_string(),
        failure_messages: if message.is_empty() {
            vec![]
        } else {
            vec![Some(format!("Error: {message}\n    at x"))]
        },
        location: None,
        not_run: false,
        expected_fail: false,
        ..Default::default()
    }
}

/// A unique, empty scratch directory (removed on drop).
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Tmp {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("turbo-test-unit")
            .join(format!("cli-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- parse_args ---------------------------------------------------------------------------

#[test]
fn parse_defaults_and_files() {
    let o = parse(&["a.test.ts", "b.test.ts"]);
    assert_eq!(
        o.files,
        vec![PathBuf::from("a.test.ts"), PathBuf::from("b.test.ts")]
    );
    assert_eq!(o.jobs, 7);
    assert_eq!(o.shard, None);
    assert_eq!(o.reporter, Reporter::Default);
    assert_eq!(o.output_file, None);
    assert_eq!(o.bail, 0);
    assert_eq!(o.allow_only, None);
    assert!(o.env.is_empty() && o.coverage.is_empty() && o.warnings.is_empty());
}

#[test]
fn parse_jobs_flags() {
    assert_eq!(parse(&["--jobs", "3"]).jobs, 3);
    assert_eq!(parse(&["-j", "4"]).jobs, 4);
    assert_eq!(parse(&["--maxWorkers", "5"]).jobs, 5);
    // unparsable value: consumed, default kept
    let o = parse(&["--jobs", "many", "x.test.ts"]);
    assert_eq!(o.jobs, 7);
    assert_eq!(o.files, vec![PathBuf::from("x.test.ts")]);
    // missing value at the end: default kept
    assert_eq!(parse(&["-j", "2", "--jobs"]).jobs, 2);
    assert_eq!(parse(&["--jobs", "3", "--no-file-parallelism"]).jobs, 1);
    // --minWorkers swallows its value (not a file); a trailing one is harmless
    let o = parse(&["--minWorkers", "4", "a.test.ts", "--minWorkers"]);
    assert_eq!(o.files, vec![PathBuf::from("a.test.ts")]);
    assert_eq!(o.jobs, 7);
}

#[test]
fn parse_timeout_retry_bail() {
    let o = parse(&["--testTimeout", "250", "--retry", "2"]);
    assert_eq!(
        o.env,
        vec![
            ("TURBO_TEST_TIMEOUT", "250".to_string()),
            ("TURBO_TEST_RETRY", "2".to_string())
        ]
    );
    // invalid numbers: value consumed, nothing set
    let o = parse(&["--testTimeout", "soon", "--retry", "-1", "f.test.ts"]);
    assert!(o.env.is_empty());
    assert_eq!(o.files, vec![PathBuf::from("f.test.ts")]);
    // missing values
    assert!(parse(&["--testTimeout"]).env.is_empty());
    assert!(parse(&["--retry"]).env.is_empty());
    assert_eq!(parse(&["--bail", "3"]).bail, 3);
    assert_eq!(parse(&["--bail", "x"]).bail, 0);
    assert_eq!(parse(&["--bail", "3", "--bail"]).bail, 0);
}

#[test]
fn parse_env_flags() {
    let o = parse(&[
        "--silent",
        "-t",
        "adds",
        "-u",
        "--testNamePattern",
        "x y",
        "--update",
    ]);
    assert_eq!(
        o.env,
        vec![
            ("TURBO_TEST_SILENT", "1".to_string()),
            ("TURBO_TEST_NAME_PATTERN", "adds".to_string()),
            ("TURBO_UPDATE_SNAPSHOTS", "1".to_string()),
            ("TURBO_TEST_NAME_PATTERN", "x y".to_string()),
            ("TURBO_UPDATE_SNAPSHOTS", "1".to_string()),
        ]
    );
    assert!(parse(&["-t"]).env.is_empty());
}

#[test]
fn parse_allow_only_toggles() {
    assert_eq!(parse(&[]).allow_only, None);
    assert_eq!(parse(&["--no-allowOnly"]).allow_only, Some(false));
    assert_eq!(
        parse(&["--no-allowOnly", "--allowOnly"]).allow_only,
        Some(true)
    );
    assert_eq!(
        parse(&["--allowOnly", "--no-allowOnly"]).allow_only,
        Some(false)
    );
}

#[test]
fn parse_shard() {
    assert_eq!(parse(&["--shard", "2/3"]).shard, Some((2, 3)));
    assert_eq!(parse(&["--shard", "a/3"]).shard, None);
    assert_eq!(parse(&["--shard", "2/b"]).shard, None);
    assert_eq!(parse(&["--shard", "23"]).shard, None);
    assert_eq!(parse(&["--shard"]).shard, None);
    // out-of-range values still parse; apply_shard ignores them
    assert_eq!(parse(&["--shard", "0/0"]).shard, Some((0, 0)));
}

#[test]
fn parse_reporters_and_output_file() {
    for (v, r) in [
        ("json", Reporter::Json),
        ("junit", Reporter::Junit),
        ("tap", Reporter::Tap),
        ("verbose", Reporter::Verbose),
        ("dot", Reporter::Dot),
        ("default", Reporter::Default),
        ("html", Reporter::Default),
    ] {
        assert_eq!(parse(&["--reporter", v]).reporter, r, "{v}");
        assert_eq!(parse(&["--reporters", v]).reporter, r, "{v}");
    }
    assert_eq!(
        parse(&["--reporter", "json", "--reporter"]).reporter,
        Reporter::Default
    );
    assert_eq!(
        parse(&["--outputFile", "o.xml"]).output_file.as_deref(),
        Some("o.xml")
    );
    assert_eq!(
        parse(&["--output-file", "p.xml"]).output_file.as_deref(),
        Some("p.xml")
    );
    assert_eq!(
        parse(&["--outputFile", "o.xml", "--outputFile"]).output_file,
        None
    );
}

#[test]
fn reporter_helpers() {
    assert_eq!(Reporter::parse(None), Reporter::Default);
    assert_eq!(Reporter::parse(Some("tap")), Reporter::Tap);
    assert!(
        Reporter::Json.is_artifact()
            && Reporter::Junit.is_artifact()
            && Reporter::Tap.is_artifact()
    );
    assert!(
        !Reporter::Default.is_artifact()
            && !Reporter::Dot.is_artifact()
            && !Reporter::Verbose.is_artifact()
    );
}

#[test]
fn parse_coverage_flags() {
    use CoverageOp::*;
    let o = parse(&[
        "--coverage",
        "--coverage-dir",
        "out",
        "--coverage-thresholds",
        "lines=90",
        "--coverage-threshold",
        "branches=1",
        "--coverage-per-file",
        "--coverage-reporter",
        "lcov",
        "--coverage-reporters",
        "text",
        "--coverage-include",
        "src/**",
        "--coverage-exclude",
        "src/x.ts",
    ]);
    assert_eq!(
        o.coverage,
        vec![
            Enable,
            Enable,
            OutDir("out".into()),
            Enable,
            Thresholds("lines=90".into()),
            Enable,
            Thresholds("branches=1".into()),
            Enable,
            PerFile,
            Enable,
            Reporters("lcov".into()),
            Enable,
            Reporters("text".into()),
            Enable,
            Include("src/**".into()),
            Enable,
            Exclude("src/x.ts".into()),
        ]
    );
    assert!(o.files.is_empty());
    // value-taking coverage flags with no value still enable coverage
    for f in [
        "--coverage-dir",
        "--coverage-thresholds",
        "--coverage-reporter",
        "--coverage-include",
        "--coverage-exclude",
    ] {
        assert_eq!(parse(&[f]).coverage, vec![Enable], "{f}");
    }
}

#[test]
fn parse_ignored_vitest_flags() {
    // value-taking no-ops consume their value
    for f in [
        "--pool",
        "--mode",
        "--project",
        "--exclude",
        "--maxConcurrency",
    ] {
        let o = parse(&[f, "v", "a.test.ts"]);
        assert_eq!(o.files, vec![PathBuf::from("a.test.ts")], "{f}");
        assert!(o.warnings.is_empty(), "{f}");
    }
    // boolean no-ops and inline/dotted forms consume nothing and warn nothing
    let o = parse(&[
        "--run",
        "--watch",
        "-w",
        "--no-watch",
        "--logHeapUsage",
        "--hideSkippedTests",
        "--no-color",
        "--color",
        "--dom",
        "--segfaultRetry",
        "--printConsoleTrace",
        "--disableConsoleIntercept",
        "--pool=forks",
        "--mode=dev",
        "--project=p",
        "--exclude=e",
        "--maxConcurrency=2",
        "--sequence.shuffle",
        "--inspect-brk",
        "--browser.name=x",
        "a.test.ts",
    ]);
    assert_eq!(o.files, vec![PathBuf::from("a.test.ts")]);
    assert!(o.warnings.is_empty());
}

#[test]
fn parse_unknown_flags_warn() {
    let o = parse(&["--frobnicate", "a.test.ts", "-z", "--silent=yes"]);
    assert_eq!(o.files, vec![PathBuf::from("a.test.ts")]);
    assert_eq!(
        o.warnings,
        vec![
            "turbo-test: ignoring unsupported flag '--frobnicate'".to_string(),
            "turbo-test: ignoring unsupported flag '-z'".to_string(),
            "turbo-test: ignoring unsupported flag '--silent=yes'".to_string(),
        ]
    );
}

#[test]
fn parse_inline_value_forms() {
    // turbo-test#32 item 2: every value flag also takes `--flag=value` (one token, nothing more
    // consumed), exactly like its space form.
    use CoverageOp::*;
    let o = parse(&[
        "--jobs=3",
        "--testTimeout=250",
        "--retry=2",
        "--bail=1",
        "--shard=1/2",
        "--reporter=json",
        "--outputFile=out.json",
        "--testNamePattern=a=b",
        "--coverage-dir=cov",
        "--coverage-thresholds=lines=90",
        "--coverage-reporter=lcov",
        "--coverage-include=src/**",
        "--coverage-exclude=x.ts",
        "--pool=forks",
        "a.test.ts",
    ]);
    assert_eq!(o.files, vec![PathBuf::from("a.test.ts")]);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_eq!(o.jobs, 3);
    assert_eq!((o.bail, o.shard), (1, Some((1, 2))));
    assert_eq!(o.reporter, Reporter::Json);
    assert_eq!(o.output_file.as_deref(), Some("out.json"));
    assert_eq!(
        o.env,
        vec![
            ("TURBO_TEST_TIMEOUT", "250".to_string()),
            ("TURBO_TEST_RETRY", "2".to_string()),
            ("TURBO_TEST_NAME_PATTERN", "a=b".to_string()),
        ]
    );
    assert_eq!(
        o.coverage,
        vec![
            Enable,
            OutDir("cov".into()),
            Enable,
            Thresholds("lines=90".into()),
            Enable,
            Reporters("lcov".into()),
            Enable,
            Include("src/**".into()),
            Enable,
            Exclude("x.ts".into()),
        ]
    );
    for (f, r) in [
        ("--reporters=tap", Reporter::Tap),
        ("--maxWorkers=5", Reporter::Default),
    ] {
        assert_eq!(parse(&[f]).reporter, r, "{f}");
    }
    assert_eq!(parse(&["--maxWorkers=5"]).jobs, 5);
    assert_eq!(
        parse(&["--output-file=p.xml"]).output_file.as_deref(),
        Some("p.xml")
    );
    // the inline value is the whole value: the next token stays a file
    let o = parse(&["--minWorkers=2", "--reporter=", "b.test.ts"]);
    assert_eq!(o.files, vec![PathBuf::from("b.test.ts")]);
    assert_eq!((o.jobs, o.reporter), (7, Reporter::Default));
    // invalid inline numbers are ignored like their space forms
    assert!(parse(&["--testTimeout=soon", "--retry=-1"]).env.is_empty());
    assert_eq!(parse(&["--bail=x"]).bail, 0);
}

#[test]
fn default_jobs_prefers_turbo_jobs() {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    assert_eq!(default_jobs(Some("3")), 3);
    assert_eq!(default_jobs(Some("lots")), cores);
    assert_eq!(default_jobs(None), cores);
}

#[test]
fn run_without_files_is_usage_error() {
    assert_eq!(run(parse_args(Vec::<String>::new(), 1)), 2);
}

// ---- sharding / durations / scheduling ----------------------------------------------------

#[test]
fn shard_partitions_by_index() {
    let files: Vec<PathBuf> = (0..7).map(|i| PathBuf::from(format!("f{i}"))).collect();
    let names = |v: Vec<PathBuf>| {
        v.into_iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(apply_shard(files.clone(), Some((1, 3)))),
        ["f0", "f3", "f6"]
    );
    assert_eq!(
        names(apply_shard(files.clone(), Some((2, 3)))),
        ["f1", "f4"]
    );
    assert_eq!(
        names(apply_shard(files.clone(), Some((3, 3)))),
        ["f2", "f5"]
    );
    assert_eq!(names(apply_shard(files.clone(), Some((1, 1)))).len(), 7);
    // invalid shards keep everything
    for s in [Some((0, 3)), Some((4, 3)), Some((1, 0)), None] {
        assert_eq!(apply_shard(files.clone(), s), files, "{s:?}");
    }
}

#[test]
fn durations_roundtrip_and_malformed_lines() {
    let t = Tmp::new("dur");
    let path = t.0.join("nested/cache/durations.tsv");
    assert!(load_durations(&path).is_empty(), "missing file → empty");
    let mut m = HashMap::new();
    m.insert("/a.test.ts".to_string(), 12.5);
    m.insert("/b c.test.ts".to_string(), 3.0);
    save_durations(&path, &m);
    assert_eq!(load_durations(&path), m);
    std::fs::write(&path, "/x\t1.5\nno-tab-line\n/y\tnot-a-number\n\n/z\t2\n").unwrap();
    let got = load_durations(&path);
    assert_eq!(got.len(), 2);
    assert_eq!(got["/x"], 1.5);
    assert_eq!(got["/z"], 2.0);
    assert!(durations_path().ends_with("turbo-test-cache/durations.tsv"));
    assert!(durations_path().starts_with(std::env::temp_dir()));
}

#[test]
fn schedule_slowest_first_ties_keep_order() {
    let files: Vec<PathBuf> = ["/a", "/b", "/c", "/d"].iter().map(PathBuf::from).collect();
    let mut hist = HashMap::new();
    hist.insert("/b".to_string(), 50.0);
    hist.insert("/d".to_string(), 900.0);
    hist.insert("zz".to_string(), 10_000.0);
    assert_eq!(schedule_order(&files, &hist), vec![3, 1, 0, 2]);
    assert_eq!(schedule_order(&files, &HashMap::new()), vec![0, 1, 2, 3]);
    assert!(schedule_order(&[], &hist).is_empty());
}

#[test]
fn non_finite_history_entries_are_ignored() {
    // turbo-test#32 item 1: `a\tNaN` parses as f64 NaN and panicked the partial_cmp sort.
    let t = Tmp::new("nan");
    let path = t.0.join("durations.tsv");
    std::fs::write(&path, "a\tNaN\nb\tinf\nc\t-inf\nd\t7\n").unwrap();
    let hist = load_durations(&path);
    assert_eq!(hist.len(), 1, "{hist:?}");
    assert_eq!(hist["d"], 7.0);
    // Even a NaN that reaches the scheduler directly sorts deterministically, never panics:
    // NaN ranks above every finite duration (total order), finite entries stay slowest-first.
    let files: Vec<PathBuf> = ["/a", "/b", "/c"].iter().map(PathBuf::from).collect();
    let mut raw = HashMap::new();
    raw.insert("/a".to_string(), 3.0);
    raw.insert("/b".to_string(), f64::NAN);
    raw.insert("/c".to_string(), 9.0);
    assert_eq!(schedule_order(&files, &raw), vec![1, 2, 0]);
}

#[test]
fn history_updates_only_files_that_ran() {
    let files: Vec<PathBuf> = ["/a", "/b"].iter().map(PathBuf::from).collect();
    let mut hist = HashMap::new();
    hist.insert("/a".to_string(), 1.0);
    hist.insert("old".to_string(), 2.0);
    let res = vec![file_result(1, &files[1], Ok(report(1, 0)), 42.0)];
    let h = update_history(hist, &files, &res);
    assert_eq!(h.len(), 3);
    assert_eq!((h["/a"], h["/b"], h["old"]), (1.0, 42.0, 2.0));
}

/// turbo-test#59: the launcher names files relative to the working directory; the JSON report
/// still names them by absolute path (vitest's `filepath`) and the duration history (shared by
/// runs from any directory) is keyed by it.
#[test]
fn json_names_and_history_keys_are_absolute() {
    let cwd = std::env::current_dir().unwrap();
    let abs = cwd.join("rel/a.test.ts").display().to_string();
    let files = vec![
        PathBuf::from("rel/a.test.ts"),
        PathBuf::from("rel/b.test.ts"),
    ];
    let res = vec![file_result(0, &files[0], Ok(report(1, 0)), 42.0)];
    let v: serde_json::Value =
        serde_json::from_str(&render_json(&files, &res, 0.0, false)).unwrap();
    assert_eq!(v["testResults"][0]["name"], abs.as_str());
    let h = update_history(HashMap::new(), &files, &res);
    assert_eq!(h.keys().collect::<Vec<_>>(), vec![&abs]);
    let mut hist = HashMap::new();
    hist.insert(cwd.join("rel/b.test.ts").display().to_string(), 5.0);
    assert_eq!(schedule_order(&files, &hist), vec![1, 0]);
}

// ---- retry decisions ----------------------------------------------------------------------

#[test]
fn transient_score_and_fail_predicates() {
    // only a file that did not run its tests is transient; no report is (turbo-test#144)
    let dom = TestReport {
        passed: 0,
        failed: 1,
        failures: vec!["t: document is not defined".into()],
        ..Default::default()
    };
    assert!(!looks_transient(&Ok(dom)));
    assert!(!looks_transient(&Ok(report(0, 2))));
    assert!(!looks_transient(&Ok(report(1, 1))));
    assert!(!looks_transient(&Ok(report(0, 0))));
    assert!(looks_transient(&Err("load".into())));
    assert!(!looks_transient(&Err(crate::heap_guard::oom_message(
        256 << 20
    ))));

    assert_eq!(score(&Ok(report(2, 3))), 1997);
    assert_eq!(score(&Err("e".into())), i64::MIN);
    assert!(score(&Ok(report(0, 5))) > score(&Err("e".into())));
}

#[test]
fn panic_payload_messages() {
    let s: Box<dyn std::any::Any + Send> = Box::new("static str");
    assert_eq!(panic_message(&*s), "static str");
    let s: Box<dyn std::any::Any + Send> = Box::new(String::from("owned"));
    assert_eq!(panic_message(&*s), "owned");
    let s: Box<dyn std::any::Any + Send> = Box::new(7u8);
    assert_eq!(panic_message(&*s), "panic");
}

/// A scripted engine: `run`/`run_fresh` pop outcomes from their queues (the last one repeats),
/// counting calls.
struct Script {
    runs: Mutex<Vec<FileOutcome>>,
    fresh: Mutex<Vec<FileOutcome>>,
    run_calls: AtomicUsize,
    fresh_calls: AtomicUsize,
    forgets: AtomicUsize,
    ends: AtomicUsize,
    reuse: bool,
}

fn clone_outcome(r: &FileOutcome) -> FileOutcome {
    match r {
        Ok(rep) => Ok(TestReport {
            passed: rep.passed,
            failed: rep.failed,
            failures: rep.failures.clone(),
            tests: rep.tests.clone(),
            suites: rep.suites.clone(),
            snapshot: rep.snapshot.clone(),
            setup_us: rep.setup_us,
            suite_errors: rep.suite_errors.clone(),
            logs: rep.logs.clone(),
            waited_ms: rep.waited_ms,
        }),
        Err(e) => Err(e.clone()),
    }
}

impl Script {
    fn new(runs: Vec<FileOutcome>, fresh: Vec<FileOutcome>, reuse: bool) -> Script {
        Script {
            runs: Mutex::new(runs),
            fresh: Mutex::new(fresh),
            run_calls: AtomicUsize::new(0),
            fresh_calls: AtomicUsize::new(0),
            forgets: AtomicUsize::new(0),
            ends: AtomicUsize::new(0),
            reuse,
        }
    }
    fn pop(q: &Mutex<Vec<FileOutcome>>) -> FileOutcome {
        let mut q = q.lock().unwrap();
        if q.len() > 1 {
            q.remove(0)
        } else {
            clone_outcome(&q[0])
        }
    }
    fn run(&self, f: impl FnOnce(&Engine) -> FileOutcome) -> FileOutcome {
        let run = |_: &Path| {
            self.run_calls.fetch_add(1, Ordering::SeqCst);
            Script::pop(&self.runs)
        };
        let fresh = |_: &Path| {
            self.fresh_calls.fetch_add(1, Ordering::SeqCst);
            Script::pop(&self.fresh)
        };
        let reuse = || self.reuse;
        let forget = || {
            self.forgets.fetch_add(1, Ordering::SeqCst);
        };
        let end = || {
            self.ends.fetch_add(1, Ordering::SeqCst);
        };
        f(&Engine {
            run: &run,
            run_fresh: &fresh,
            reuse_enabled: &reuse,
            forget_registry: &forget,
            end_worker: &end,
        })
    }
    fn calls(&self) -> (usize, usize) {
        (
            self.run_calls.load(Ordering::SeqCst),
            self.fresh_calls.load(Ordering::SeqCst),
        )
    }
}

fn exec(s: &Script) -> FileOutcome {
    s.run(|e| execute_file(e, Path::new("f.test.ts")))
}

#[test]
fn passing_file_runs_once() {
    let s = Script::new(vec![Ok(ran(2, &[]))], vec![Ok(report(9, 0))], true);
    assert_eq!(exec(&s).unwrap().passed, 2);
    assert_eq!(s.calls(), (1, 0));
}

#[test]
fn partial_failure_is_not_retried_without_reuse() {
    let s = Script::new(vec![Ok(report(1, 1))], vec![], false);
    assert_eq!(exec(&s).unwrap().failed, 1);
    assert_eq!(s.calls(), (1, 0));
}

#[test]
fn load_error_retries_keep_the_best_attempt() {
    // a later report beats the load error; the loop stops at the first report
    let s = Script::new(
        vec![Err("load".into()), Ok(report(0, 1)), Ok(report(5, 0))],
        vec![],
        false,
    );
    assert_eq!(exec(&s).unwrap().failed, 1);
    assert_eq!(s.calls(), (2, 0));
    // three load errors: the first is kept
    let s = Script::new(
        vec![Err("first".into()), Err("second".into())],
        vec![],
        false,
    );
    assert_eq!(exec(&s).unwrap_err(), "first");
    assert_eq!(s.calls(), (3, 0));
}

#[test]
fn load_error_retried_then_reported() {
    let s = Script::new(vec![Err("graph load failed".into())], vec![], false);
    assert_eq!(exec(&s).unwrap_err(), "graph load failed");
    assert_eq!(s.calls(), (3, 0));
    let s = Script::new(vec![Err("flaky".into()), Ok(report(3, 0))], vec![], false);
    assert_eq!(exec(&s).unwrap().passed, 3);
    assert_eq!(s.calls(), (2, 0));
}

#[test]
fn panics_become_load_errors_and_forget_registry() {
    let forgets = AtomicUsize::new(0);
    let calls = AtomicUsize::new(0);
    let run = |_: &Path| -> FileOutcome {
        match calls.fetch_add(1, Ordering::SeqCst) {
            0 => panic!("boom in loader"),
            1 => std::panic::panic_any(String::from("owned payload")),
            _ => std::panic::panic_any(42i32),
        }
    };
    let forget = || {
        forgets.fetch_add(1, Ordering::SeqCst);
    };
    let e = Engine {
        run: &run,
        forget_registry: &forget,
        reuse_enabled: &|| false,
        ..Engine::v8()
    };
    // all three attempts panic; every outcome is Err (score MIN), so the first is kept
    assert_eq!(
        execute_file(&e, Path::new("p.test.ts")).unwrap_err(),
        "panicked: boom in loader"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(forgets.load(Ordering::SeqCst), 3);
}

#[test]
fn panic_recovery_with_the_real_registry_hook() {
    // the real forget_registry is safe on a thread that never built an isolate
    let calls = AtomicUsize::new(0);
    let run = |_: &Path| -> FileOutcome {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("first attempt dies");
        }
        Ok(report(1, 0))
    };
    let e = Engine {
        run: &run,
        reuse_enabled: &|| false,
        ..Engine::v8()
    };
    assert_eq!(execute_file(&e, Path::new("p.test.ts")).unwrap().passed, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn reuse_error_adopts_fresh_result() {
    // a load error under reuse → one fresh run, adopted
    let s = Script::new(vec![Err("load".into())], vec![Ok(report(3, 0))], true);
    assert_eq!(exec(&s).unwrap().passed, 3);
    assert_eq!(s.calls(), (3, 1));
    // fresh is authoritative even when it fails
    let s = Script::new(vec![Err("load".into())], vec![Ok(report(1, 2))], true);
    let r = exec(&s).unwrap();
    assert_eq!((r.passed, r.failed), (1, 2));
    assert_eq!(s.calls(), (3, 1));
    // a fresh load error is re-run up to twice more, stopping at the first report
    let s = Script::new(
        vec![Err("load".into())],
        vec![Err("x".into()), Ok(report(0, 1)), Ok(report(0, 3))],
        true,
    );
    assert_eq!(exec(&s).unwrap().failed, 1);
    assert_eq!(s.calls(), (3, 2));
    let s = Script::new(vec![Err("load".into())], vec![Err("x".into())], true);
    assert_eq!(exec(&s).unwrap_err(), "x");
    assert_eq!(s.calls(), (3, 3));
    let s = Script::new(
        vec![Err("load".into())],
        vec![Err("x".into()), Ok(report(4, 0))],
        true,
    );
    assert_eq!(exec(&s).unwrap().passed, 4);
    assert_eq!(s.calls(), (3, 2));
}

/// A report whose `passed` tests pass and whose `failed` tests fail with errors named `errors`
/// (one name per failed test).
fn ran(passed: u32, errors: &[&str]) -> TestReport {
    let mut tests: Vec<TestCase> = (0..passed)
        .map(|i| case(&format!("p{i}"), "passed", 1.0, ""))
        .collect();
    for (i, name) in errors.iter().enumerate() {
        let mut t = case(&format!("f{i}"), "failed", 1.0, "boom");
        t.errors = vec![crate::failure::ErrorInfo {
            name: name.to_string(),
            message: "boom".into(),
            ..Default::default()
        }];
        tests.push(t);
    }
    TestReport {
        passed,
        failed: errors.len() as u32,
        failures: errors.iter().map(|n| format!("f: {n}: boom")).collect(),
        tests,
        ..Default::default()
    }
}

#[test]
fn reuse_result_finality() {
    // final: the tests ran and every failure is an assertion failure (what vitest reports)
    assert!(reuse_result_is_final(&Ok(ran(2, &[]))));
    assert!(reuse_result_is_final(&Ok(ran(1, &["AssertionError"]))));
    assert!(reuse_result_is_final(&Ok(ran(
        0,
        &["AssertionError", "AssertionError"]
    ))));
    let mut skipped = ran(0, &[]);
    skipped.tests.push(case("s", "skipped", 0.0, ""));
    assert!(reuse_result_is_final(&Ok(skipped)));
    // not final: a non-assertion error, a failure without an error, a failed hook, no collected
    // test, "no test suite", a load error / panic / out of memory
    assert!(!reuse_result_is_final(&Ok(ran(
        1,
        &["AssertionError", "TypeError"]
    ))));
    let mut bare = ran(0, &["AssertionError"]);
    bare.tests[0].errors.clear();
    assert!(!reuse_result_is_final(&Ok(bare)));
    let mut miscounted = ran(1, &["AssertionError"]);
    miscounted.failed = 2;
    assert!(!reuse_result_is_final(&Ok(miscounted)));
    let mut hook = ran(1, &[]);
    hook.suite_errors.push(Default::default());
    assert!(!reuse_result_is_final(&Ok(hook)));
    assert!(!reuse_result_is_final(&Ok(report(0, 0))));
    let mut none = ran(1, &[]);
    none.failures.push(format!("{NO_SUITE_FOUND}/p/a.test.ts"));
    assert!(!reuse_result_is_final(&Ok(none)));
    assert!(!reuse_result_is_final(&Err("load".into())));
    assert!(!reuse_result_is_final(&Err(
        crate::heap_guard::oom_message(256 << 20)
    )));
}

#[test]
fn reuse_keeps_a_final_report_and_retries_the_rest_fresh() {
    // vitest isolate:false runs a file once on the shared worker and reports that run: a final
    // report (assertion failures, all passing, all-failing included) is adopted with no other
    // run, on the reused isolate or a fresh one (turbo-test#126)
    for errors in [
        &[][..],
        &["AssertionError"][..],
        &["AssertionError", "AssertionError"][..],
    ] {
        for passed in [0, 2] {
            if passed == 0 && errors.is_empty() {
                continue;
            }
            let s = Script::new(vec![Ok(ran(passed, errors))], vec![Ok(report(9, 0))], true);
            let r = exec(&s).unwrap();
            assert_eq!((r.passed, r.failed), (passed, errors.len() as u32));
            assert_eq!(s.calls(), (1, 0));
        }
    }
    // a test failing with a non-assertion error may be a reuse artifact: one fresh run, adopted,
    // and never a second run on the reused isolate (its module is already evaluated there)
    let s = Script::new(
        vec![Ok(ran(0, &["TypeError"]))],
        vec![Ok(report(1, 0))],
        true,
    );
    assert_eq!(exec(&s).unwrap().passed, 1);
    assert_eq!(s.calls(), (1, 1));
    // a file that did not run its tests is re-run fresh (a load error after its transient retries)
    let s = Script::new(
        vec![Err("graph load failed".into())],
        vec![Ok(report(4, 0))],
        true,
    );
    assert_eq!(exec(&s).unwrap().passed, 4);
    assert_eq!(s.calls(), (3, 1));
    let oom = crate::heap_guard::oom_message(256 << 20);
    let s = Script::new(vec![Err(oom.clone())], vec![Ok(report(1, 0))], true);
    assert_eq!(exec(&s).unwrap().passed, 1);
    // J2 (turbo-test#124): out of memory is not retried on the same kind of isolate
    assert_eq!(s.calls(), (1, 1));
    let s = Script::new(vec![Err(oom.clone())], vec![Err(oom.clone())], true);
    assert_eq!(exec(&s).unwrap_err(), oom);
    assert_eq!(s.calls(), (1, 1));
    // without reuse nothing changes: a final-looking failing report is kept, run once
    let s = Script::new(vec![Ok(ran(1, &["AssertionError"]))], vec![], false);
    assert_eq!(exec(&s).unwrap().failed, 1);
    assert_eq!(s.calls(), (1, 0));
}

#[test]
fn a_report_is_never_rerun_on_the_same_kind_of_isolate() {
    // vitest runs a file once: a file whose tests all fail (or one that mentions an undefined
    // `document`) is reported from its one run, with fresh isolates too (turbo-test#144)
    let mut dom = report(0, 1);
    dom.failures = vec!["t: ReferenceError: document is not defined".into()];
    for first in [report(0, 2), dom] {
        let s = Script::new(vec![Ok(first), Ok(report(5, 0))], vec![], false);
        assert_eq!(exec(&s).unwrap().passed, 0);
        assert_eq!(s.calls(), (1, 0));
    }
}

// ---- file results / worker pool -----------------------------------------------------------

#[test]
fn file_result_lines() {
    let f = Path::new("/p/a.test.ts");
    let r = file_result(
        3,
        f,
        Ok(TestReport {
            setup_us: 7.0,
            ..report(2, 0)
        }),
        9.5,
    );
    assert_eq!(r.line, "PASS  /p/a.test.ts  (2 passed, 0 failed)");
    assert_eq!(
        (
            r.idx,
            r.passed,
            r.failed,
            r.load_error,
            r.setup_us,
            r.dur_ms
        ),
        (3, 2, 0, false, 7.0, 9.5)
    );
    assert!(r.error_msg.is_empty());
    let rep = TestReport {
        failures: vec!["x: boom".into(), "y: bang".into()],
        tests: vec![case("x", "failed", 1.0, "boom")],
        ..report(1, 2)
    };
    let r = file_result(0, f, Ok(rep), 1.0);
    assert_eq!(
        r.line,
        "FAIL  /p/a.test.ts  (1 passed, 2 failed)\n        ✗ x: boom\n        ✗ y: bang"
    );
    assert_eq!(r.tests.len(), 1);
    // only skipped / name-filtered tests: nothing failed → PASS, matching how it is counted
    // (turbo-test#32 item 7: it printed FAIL yet counted as a passed file)
    let r = file_result(0, f, Ok(report(0, 0)), 1.0);
    assert_eq!(r.line, "PASS  /p/a.test.ts  (0 passed, 0 failed)");
    assert!(!r.load_error);
    // a file that collected nothing at all: the runtime's vitest "No test suite found in file"
    // report becomes a file-level error (like a load error) so every reporter fails it
    let msg = "No test suite found in file /p/a.test.ts";
    let rep = TestReport {
        failures: vec![msg.into()],
        ..report(0, 0)
    };
    let r = file_result(4, f, Ok(rep), 3.0);
    assert_eq!(
        r.line,
        "ERROR /p/a.test.ts  (No test suite found in file /p/a.test.ts)"
    );
    assert!(r.load_error && r.tests.is_empty());
    assert_eq!((r.idx, r.passed, r.failed, r.dur_ms), (4, 0, 0, 3.0));
    assert_eq!(r.error_msg, msg);
    // the same text as a real failure (failed > 0) stays an ordinary FAIL
    let rep = TestReport {
        failures: vec![msg.into()],
        ..report(0, 1)
    };
    assert!(file_result(0, f, Ok(rep), 1.0).line.starts_with("FAIL  "));
    let r = file_result(1, f, Err("graph load failed".into()), 2.0);
    assert_eq!(r.line, "ERROR /p/a.test.ts  (graph load failed)");
    assert!(r.load_error && r.tests.is_empty());
    assert_eq!((r.passed, r.failed, r.setup_us, r.dur_ms), (0, 0, 0.0, 2.0));
    assert_eq!(r.error_msg, "graph load failed");
}

/// Engine whose outcome depends on the file name: `pass*` → 1 passed, `fail*` → 1 failed.
fn by_name(f: &Path) -> FileOutcome {
    let n = f.file_name().unwrap().to_string_lossy();
    if n.starts_with("pass") {
        Ok(report(1, 0))
    } else if n.starts_with("fail") {
        Ok(report(0, 1))
    } else {
        Err(format!("cannot load {n}"))
    }
}

#[test]
fn worker_pool_runs_everything_sorted_by_index() {
    let files: Vec<PathBuf> = ["pass1", "fail1", "err1", "pass2", "pass3"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let ends = AtomicUsize::new(0);
    let end = || {
        ends.fetch_add(1, Ordering::SeqCst);
    };
    let e = Engine {
        run: &by_name,
        reuse_enabled: &|| false,
        end_worker: &end,
        ..Engine::v8()
    };
    let res = run_files(&e, &files, &files, &[4, 3, 2, 1, 0], 3, 0);
    assert_eq!(
        res.iter().map(|r| r.idx).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(res[1].failed, 1);
    assert!(res[2].load_error);
    assert_eq!(ends.load(Ordering::SeqCst), 3, "every worker tears down");
    let t = tally(&res);
    assert_eq!((t.passed, t.failed, t.errors, t.setup_n), (3, 1, 1, 4));
}

#[test]
fn bail_stops_pulling_new_files() {
    let files: Vec<PathBuf> = ["fail1", "fail2", "pass1", "fail3"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let seen = Mutex::new(Vec::new());
    let run = |f: &Path| {
        seen.lock().unwrap().push(f.display().to_string());
        by_name(f)
    };
    let e = Engine {
        run: &run,
        reuse_enabled: &|| false,
        end_worker: &|| {},
        ..Engine::v8()
    };
    // bail 2, one worker, slow-first order 2,0,1,3: pass1, fail1 (x3 retries), fail2 → stop
    let res = run_files(&e, &files, &files, &[2, 0, 1, 3], 1, 2);
    assert_eq!(res.iter().map(|r| r.idx).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|f| *f == "fail3")
            .count(),
        0
    );
    assert_eq!(seen.lock().unwrap()[0], "pass1", "order is honoured");
    // bail 0 = disabled
    assert_eq!(run_files(&e, &files, &files, &[0, 1, 2, 3], 1, 0).len(), 4);
    // nothing to run
    assert!(run_files(&e, &[], &[], &[], 1, 0).is_empty());
}

// ---- renderers ----------------------------------------------------------------------------

fn sample() -> (Vec<PathBuf>, Vec<FileResult>) {
    let files: Vec<PathBuf> = ["/p/a.test.ts", "/p/b\\\"q.test.ts", "/p/c&<x>.test.ts"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let a = TestReport {
        tests: vec![
            case("suite > adds", "passed", 1.4, ""),
            case("suite > skipped one", "skipped", 0.0, ""),
            case(
                "suite > breaks <&>",
                "failed",
                12.6,
                "expected \"1\"\nto be '2'",
            ),
            case("todo", "todo", 0.0, ""),
            case("silent fail", "failed", 0.0, ""),
        ],
        failures: vec!["suite > breaks <&>: expected".into()],
        setup_us: 1500.0,
        ..report(2, 2)
    };
    let b = TestReport {
        tests: vec![case("ok", "passed", 2.0, "")],
        setup_us: 500.0,
        ..report(1, 0)
    };
    let res = vec![
        file_result(0, &files[0], Ok(a), 1234.0),
        file_result(1, &files[1], Ok(b), 20.0),
        file_result(
            2,
            &files[2],
            Err("module errored: Error: top <boom> & \"q\"".into()),
            5.0,
        ),
    ];
    (files, res)
}

#[test]
fn human_reporters() {
    let (_, res) = sample();
    let lines = format!("{}\n{}\n{}\n", res[0].line, res[1].line, res[2].line);
    assert_eq!(render_human(Reporter::Default, &res), lines);
    assert_eq!(render_human(Reporter::Dot, &res), "x.!\n");
    assert_eq!(render_human(Reporter::Dot, &[]), "\n");
    let v = render_human(Reporter::Verbose, &res);
    let expected = format!(
        "{}\n   ✓ suite > adds (1ms)\n   - suite > skipped one (0ms)\n   ✗ suite > breaks <&> (13ms)\n   - todo (0ms)\n   ✗ silent fail (0ms)\n{}\n   ✓ ok (2ms)\n{}\n",
        res[0].line, res[1].line, res[2].line
    );
    assert_eq!(v, expected);
    for r in [Reporter::Json, Reporter::Junit, Reporter::Tap] {
        assert_eq!(render_human(r, &res), "", "{r:?}");
    }
}

#[test]
fn vitests_human_reporter_kinds() {
    use crate::human::Kind;
    let kind = |r, explicit, legacy, agent| human_kind_for(r, explicit, legacy, agent);
    assert_eq!(
        kind(Reporter::Default, false, false, false),
        Some(Kind::Default)
    );
    assert_eq!(
        kind(Reporter::Default, true, false, false),
        Some(Kind::Default)
    );
    // an agent gets the agent reporter only without --reporter
    assert_eq!(
        kind(Reporter::Default, false, false, true),
        Some(Kind::Agent)
    );
    assert_eq!(
        kind(Reporter::Default, true, false, true),
        Some(Kind::Default)
    );
    assert_eq!(
        kind(Reporter::Verbose, false, false, true),
        Some(Kind::Verbose)
    );
    // vitest's DotReporter (turbo-test#100), with or without an agent
    assert_eq!(kind(Reporter::Dot, true, false, false), Some(Kind::Dot));
    assert_eq!(kind(Reporter::Dot, true, false, true), Some(Kind::Dot));
    for r in [Reporter::Json, Reporter::Junit, Reporter::Tap] {
        assert_eq!(kind(r, true, false, false), None, "{r:?}");
    }
    // TURBO_LEGACY_REPORTER: turbo-test's own lines for every reporter
    assert_eq!(kind(Reporter::Default, false, true, true), None);
    assert_eq!(kind(Reporter::Verbose, true, true, false), None);
    assert_eq!(kind(Reporter::Dot, true, true, false), None);
}

#[test]
fn local_times_are_hh_mm_ss() {
    let secs = |hms: &str| -> i64 {
        let p: Vec<i64> = hms.split(':').map(|x| x.parse().unwrap()).collect();
        assert_eq!((p.len(), hms.len()), (3, 8), "{hms}");
        p[0] * 3600 + p[1] * 60 + p[2]
    };
    let t = 1_700_000_000_000.0;
    // 61s and a fraction later, whatever the zone
    assert_eq!(
        (secs(&local_hms(t + 61_900.0)) - secs(&local_hms(t))).rem_euclid(86_400),
        61
    );
}

#[test]
fn xml_escaping() {
    assert_eq!(
        xml_esc("a&b<c>d\"e'f é"),
        "a&amp;b&lt;c&gt;d&quot;e&apos;f é"
    );
    assert_eq!(xml_esc(""), "");
}

#[test]
fn json_reporter() {
    let (files, mut res) = sample();
    res[0].suites = vec!["fail".into(), "todo".into(), "skip".into(), "pass".into()];
    let j = render_json(&files, &res, 500.5, true);
    // vitest 4.1.12's shape and key order: files + describes are the suites (a failed file, a
    // failed and a todo describe), skipped / todo tests have no duration, a load error's message
    // drops turbo-test's prefix and the error name, start / end span the tests that ran.
    let snap = "{\"added\":0,\"failure\":false,\"filesAdded\":0,\"filesRemoved\":0,\"filesRemovedList\":[],\"filesUnmatched\":0,\"filesUpdated\":0,\"matched\":0,\"total\":0,\"unchecked\":0,\"uncheckedKeysByFile\":[],\"unmatched\":0,\"updated\":0,\"didUpdate\":true}";
    let a = [
        "{\"ancestorTitles\":[\"suite\"],\"fullName\":\"suite adds\",\"status\":\"passed\",\"title\":\"adds\",\"duration\":1.4,\"failureMessages\":[],\"meta\":{},\"tags\":[]}",
        "{\"ancestorTitles\":[\"suite\"],\"fullName\":\"suite skipped one\",\"status\":\"skipped\",\"title\":\"skipped one\",\"failureMessages\":[],\"meta\":{},\"tags\":[]}",
        "{\"ancestorTitles\":[\"suite\"],\"fullName\":\"suite breaks <&>\",\"status\":\"failed\",\"title\":\"breaks <&>\",\"duration\":12.6,\"failureMessages\":[\"Error: expected \\\"1\\\"\\nto be '2'\\n    at x\"],\"meta\":{},\"tags\":[]}",
        "{\"ancestorTitles\":[],\"fullName\":\"todo\",\"status\":\"todo\",\"title\":\"todo\",\"failureMessages\":[],\"meta\":{},\"tags\":[]}",
        "{\"ancestorTitles\":[],\"fullName\":\"silent fail\",\"status\":\"failed\",\"title\":\"silent fail\",\"duration\":0,\"failureMessages\":[],\"meta\":{},\"tags\":[]}",
    ]
    .join(",");
    let b = "{\"ancestorTitles\":[],\"fullName\":\"ok\",\"status\":\"passed\",\"title\":\"ok\",\"duration\":2,\"failureMessages\":[],\"meta\":{},\"tags\":[]}";
    assert_eq!(
        j,
        format!(
            "{{\"numTotalTestSuites\":7,\"numPassedTestSuites\":3,\"numFailedTestSuites\":3,\"numPendingTestSuites\":1,\"numTotalTests\":6,\"numPassedTests\":2,\"numFailedTests\":2,\"numPendingTests\":1,\"numTodoTests\":1,\"snapshot\":{snap},\"startTime\":500.5,\"success\":false,\"testResults\":[\
             {{\"assertionResults\":[{a}],\"startTime\":1000,\"endTime\":1012.6,\"status\":\"failed\",\"message\":\"\",\"name\":\"/p/a.test.ts\"}},\
             {{\"assertionResults\":[{b}],\"startTime\":1000,\"endTime\":1002,\"status\":\"passed\",\"message\":\"\",\"name\":\"/p/b\\\\\\\"q.test.ts\"}},\
             {{\"assertionResults\":[],\"startTime\":500.5,\"endTime\":500.5,\"status\":\"failed\",\"message\":\"top <boom> & \\\"q\\\"\",\"name\":\"/p/c&<x>.test.ts\"}}]}}"
        )
    );
    // success: some file ran, no failed suite, no failed test
    let v: serde_json::Value =
        serde_json::from_str(&render_json(&files, &res[1..2], 0.0, false)).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["snapshot"]["didUpdate"], false);
    let v: serde_json::Value = serde_json::from_str(&render_json(&[], &[], 0.0, false)).unwrap();
    assert_eq!(v["success"], false);
    assert_eq!(v["numTotalTestSuites"], 0);
}

#[test]
fn json_file_messages_and_numbers() {
    // the load-error message vitest shows is the error's own message
    for (err, want) in [
        ("module errored: Error: top", "top"),
        (
            "evaluate threw: TypeError: x is not a function",
            "x is not a function",
        ),
        ("instantiate: SyntaxError: bad", "bad"),
        ("graph load failed", "graph load failed"),
        (
            "No test suite found in file /p/a.test.ts",
            "No test suite found in file /p/a.test.ts",
        ),
        ("module errored: Boom: kept", "Boom: kept"),
        ("module errored: Weird Error: kept", "Weird Error: kept"),
    ] {
        assert_eq!(file_message(err), want, "{err}");
    }
    assert_eq!(js_num(3.0), "3");
    assert_eq!(js_num(-0.0), "0");
    assert_eq!(js_num(0.25), "0.25");
    assert_eq!(js_num(1e21), "1000000000000000000000");
}

#[test]
fn junit_reporter() {
    let (files, res) = sample();
    let x = junit_norm(&render_junit(&files, &res, Path::new("/p")));
    // vitest 4.1.12's junit layout (turbo-test#81); a failed test with no error object (the
    // fixture's) gets an `Error` failure from its message
    let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<testsuites name=\"vitest tests\" tests=\"7\" failures=\"3\" errors=\"0\" time=\"1.254\">\n    <testsuite name=\"/p/a.test.ts\" timestamp=\"S\" hostname=\"H\" tests=\"5\" failures=\"2\" errors=\"0\" skipped=\"2\" time=\"1.234\">\n        <testcase classname=\"/p/a.test.ts\" name=\"suite &gt; adds\" time=\"0.0014\">\n        </testcase>\n        <testcase classname=\"/p/a.test.ts\" name=\"suite &gt; skipped one\" time=\"0\">\n            <skipped/>\n        </testcase>\n        <testcase classname=\"/p/a.test.ts\" name=\"suite &gt; breaks &lt;&amp;&gt;\" time=\"0.0126\">\n            <failure message=\"expected &quot;1&quot;\nto be &apos;2&apos;\" type=\"Error\">\nError: expected &quot;1&quot;\nto be &apos;2&apos;\n            </failure>\n        </testcase>\n        <testcase classname=\"/p/a.test.ts\" name=\"todo\" time=\"0\">\n            <skipped/>\n        </testcase>\n        <testcase classname=\"/p/a.test.ts\" name=\"silent fail\" time=\"0\">\n            <failure message=\"\" type=\"Error\">\nError:\n            </failure>\n        </testcase>\n    </testsuite>\n    <testsuite name=\"/p/b\\&quot;q.test.ts\" timestamp=\"S\" hostname=\"H\" tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"0\" time=\"0.02\">\n        <testcase classname=\"/p/b\\&quot;q.test.ts\" name=\"ok\" time=\"0.002\">\n        </testcase>\n    </testsuite>\n    <testsuite name=\"/p/c&amp;&lt;x&gt;.test.ts\" timestamp=\"S\" hostname=\"H\" tests=\"1\" failures=\"1\" errors=\"0\" skipped=\"0\" time=\"0\">\n        <testcase classname=\"/p/c&amp;&lt;x&gt;.test.ts\" name=\"/p/c&amp;&lt;x&gt;.test.ts\" time=\"0\">\n            <failure message=\"top &lt;boom&gt; &amp; &quot;q&quot;\" type=\"Error\">\nError: top &lt;boom&gt; &amp; &quot;q&quot;\n            </failure>\n        </testcase>\n    </testsuite>\n</testsuites>";
    assert_eq!(x, expected);
}

#[test]
fn tap_reporter() {
    let (files, res) = sample();
    // turbo-test#32 item 4: the load-error file (`files[2]`) is one `not ok` point carrying the
    // error, like JUnit's / JSON's synthetic failing case — it was silently dropped.
    let expected = "TAP version 13\n1..7\n\
ok 1 - suite > adds\n\
ok 2 - suite > skipped one # SKIP\n\
not ok 3 - suite > breaks <&>\n  ---\n  message: expected \"1\" to be '2'\n  ...\n\
ok 4 - todo\n\
not ok 5 - silent fail\n\
ok 6 - ok\n\
not ok 7 - /p/c&<x>.test.ts\n  ---\n  message: module errored: Error: top <boom> & \"q\"\n  ...\n";
    assert_eq!(render_tap(&files, &res), expected);
    assert_eq!(render_tap(&[], &[]), "TAP version 13\n1..0\n");
    // a multi-line load error is flattened like a test failure message
    let only_err = vec![file_result(0, &files[0], Err("a\nb".into()), 1.0)];
    assert_eq!(
        render_tap(&files, &only_err),
        "TAP version 13\n1..1\nnot ok 1 - /p/a.test.ts\n  ---\n  message: a b\n  ...\n"
    );
}

#[test]
fn artifact_dispatch() {
    let (files, res) = sample();
    assert_eq!(
        render_artifact(Reporter::Json, &files, &files, &res, 7.0, true),
        Some(render_json(&files, &res, 7.0, true))
    );
    assert_eq!(
        render_artifact(Reporter::Junit, &files, &files, &res, 0.0, false).map(|x| junit_norm(&x)),
        Some(junit_norm(&render_junit(&files, &res, &junit_root())))
    );
    assert_eq!(
        render_artifact(Reporter::Tap, &files, &files, &res, 0.0, false),
        Some(render_tap(&files, &res))
    );
    for r in [Reporter::Default, Reporter::Dot, Reporter::Verbose] {
        assert_eq!(render_artifact(r, &files, &files, &res, 0.0, false), None);
    }
}

#[test]
fn output_delivery() {
    let (_, res) = sample();
    let t = Tmp::new("out");
    // artifact without --outputFile → stdout (+ newline)
    assert_eq!(
        emit_output(Some("ART"), Reporter::Json, None, &res),
        ("ART\n".to_string(), String::new())
    );
    // artifact → file, parent dirs created; vitest's junit / json say where it went (junit
    // writes each line with its newline, json one line without), tap says nothing
    let p = t.0.join("deep/er/report.xml");
    let ps = p.to_str().unwrap();
    assert_eq!(
        emit_output(Some("<x/>"), Reporter::Junit, Some(ps), &res),
        (format!("JUNIT report written to {ps}\n"), String::new())
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "<x/>\n");
    let j = t.0.join("r.json");
    let js = j.to_str().unwrap();
    assert_eq!(
        emit_output(Some("{}"), Reporter::Json, Some(js), &res),
        (format!("JSON report written to {js}\n"), String::new())
    );
    assert_eq!(std::fs::read_to_string(&j).unwrap(), "{}");
    let tap = t.0.join("r.tap");
    assert_eq!(
        emit_output(Some("TAP"), Reporter::Tap, tap.to_str(), &res),
        (String::new(), String::new())
    );
    assert_eq!(std::fs::read_to_string(&tap).unwrap(), "TAP");
    // unwritable target (a directory) → stderr message, no stdout
    let dir = t.0.to_str().unwrap();
    let (out, err) = emit_output(Some("x"), Reporter::Tap, Some(dir), &res);
    assert_eq!(out, "");
    assert!(
        err.starts_with(&format!(
            "turbo-test: failed to write --outputFile '{dir}': "
        )),
        "{err}"
    );
    assert!(err.ends_with('\n'));
    // human reporter + --outputFile → the PASS/FAIL lines
    let txt = t.0.join("lines.txt");
    let ts = txt.to_str().unwrap();
    assert_eq!(
        emit_output(None, Reporter::Verbose, Some(ts), &res),
        (String::new(), String::new())
    );
    let lines = format!("{}\n{}\n{}\n", res[0].line, res[1].line, res[2].line);
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), lines);
    // human reporter, no file → nothing
    assert_eq!(
        emit_output(None, Reporter::Default, None, &res),
        (String::new(), String::new())
    );
    // turbo-test#32 item 5: the human-reporter branch creates parent dirs like the artifact
    // branch, and reports a failed write instead of dropping it silently
    let deep = t.0.join("no/such/dir/out.txt");
    assert_eq!(
        emit_output(None, Reporter::Default, deep.to_str(), &res),
        (String::new(), String::new())
    );
    assert_eq!(std::fs::read_to_string(&deep).unwrap(), lines);
    let (out, err) = emit_output(None, Reporter::Dot, Some(dir), &res);
    assert_eq!(out, "");
    assert!(
        err.starts_with(&format!(
            "turbo-test: failed to write --outputFile '{dir}': "
        )),
        "{err}"
    );
}

#[test]
fn footer_summary_and_exit_code() {
    assert_eq!(format_duration(999.4), "999ms");
    assert_eq!(format_duration(1000.0), "1.00s");
    assert_eq!(format_duration(12345.0), "12.35s");
    // vitest's getStateString (turbo-test#78)
    assert_eq!(state_string([0, 4, 0, 0, 0], 4), "4 passed (4)");
    assert_eq!(state_string([1, 3, 0, 0, 0], 5), "1 failed | 3 passed (5)");
    assert_eq!(state_string([2, 0, 0, 0, 0], 2), "2 failed (2)");
    assert_eq!(
        state_string([1, 1, 1, 2, 3], 9),
        "1 failed | 1 passed | 1 expected fail | 2 skipped | 3 todo (9)"
    );
    assert_eq!(state_string([0, 0, 0, 0, 0], 2), " (2)");
    assert_eq!(state_string([0, 0, 0, 0, 0], 0), "no tests");
    // vitest's colours: failed / passed bold red / green, expected fail cyan, skipped yellow,
    // todo and the total gray, the separators dim; `no tests` dim
    let c = crate::colors::Colors { on: true };
    assert_eq!(
        state_string_colored([1, 2, 3, 4, 5], 15, &c),
        "\x1b[1m\x1b[31m1 failed\x1b[39m\x1b[22m\x1b[2m | \x1b[22m\x1b[1m\x1b[32m2 passed\x1b[39m\x1b[22m\x1b[2m | \x1b[22m\x1b[36m3 expected fail\x1b[39m\x1b[2m | \x1b[22m\x1b[33m4 skipped\x1b[39m\x1b[2m | \x1b[22m\x1b[90m5 todo\x1b[39m\x1b[90m (15)\x1b[39m"
    );
    assert_eq!(
        state_string_colored([0; 5], 0, &c),
        "\x1b[2mno tests\x1b[22m"
    );

    let (_, res) = sample();
    let t = tally(&res);
    assert_eq!(
        footer(4, &res, 250.0),
        "\n Test Files  2 failed | 1 passed (4)\n      Tests  2 failed | 2 passed | 1 skipped | 1 todo (6)\n   Duration  250ms"
    );
    assert_eq!(
        summary(4, &t, 2, 250.4, (3, 1)),
        "\n4 files | 3 passed | 2 failed | 1 load-errors | 2 jobs | wall 250 ms | env setup 1.00 ms/file | cache 75% hit"
    );
    assert_eq!(
        summary(0, &Totals::default(), 1, 0.0, (0, 0)),
        "\n0 files | 0 passed | 0 failed | 0 load-errors | 1 jobs | wall 0 ms | env setup 0.00 ms/file | cache 0% hit"
    );
    // a pending test (a `.only` inside describe.skip, turbo-test#77) counts only in the total, and
    // a file none of whose tests ran is skipped (vitest: `Tests  1 skipped (2)`)
    let pending = TestReport {
        tests: vec![
            case("k > w", "pending", 0.0, ""),
            case("k > v", "skipped", 0.0, ""),
        ],
        ..report(0, 0)
    };
    let res = vec![file_result(0, Path::new("k.test.js"), Ok(pending), 1.0)];
    assert_eq!(
        footer(1, &res, 1.0),
        "\n Test Files  1 skipped (1)\n      Tests  1 skipped (2)\n   Duration  1ms"
    );
    assert_eq!(
        footer(0, &[], 0.0),
        "\n Test Files  no tests\n      Tests  no tests\n   Duration  0ms"
    );

    assert_eq!(exit_code(&Totals::default(), true), 0);
    assert_eq!(exit_code(&Totals::default(), false), 1);
    assert_eq!(
        exit_code(
            &Totals {
                failed: 1,
                ..Default::default()
            },
            true
        ),
        1
    );
    assert_eq!(
        exit_code(
            &Totals {
                errors: 1,
                ..Default::default()
            },
            true
        ),
        1
    );
    // a suite / file-level error (a throwing beforeAll, an empty describe) with no failed test
    assert_eq!(
        exit_code(
            &Totals {
                failed_files: 1,
                ..Default::default()
            },
            true
        ),
        1
    );
}

#[test]
fn a_suite_error_without_a_failed_test_fails_the_file() {
    // turbo-test#69: a throwing beforeAll / an empty describe is a file-level failure with no
    // failed test — the file fails (FAIL line, dot `x`, footer, exit 1) and no test is invented.
    let file = PathBuf::from("a.test.ts");
    let rep = TestReport {
        failures: vec!["No test found in suite empty".into()],
        tests: vec![case("ok", "passed", 1.0, "")],
        ..report(1, 0)
    };
    let res = vec![file_result(0, &file, Ok(rep), 1.0)];
    assert!(res[0].failed_file && !res[0].load_error);
    assert_eq!(
        res[0].line,
        "FAIL  a.test.ts  (1 passed, 0 failed)\n        ✗ No test found in suite empty"
    );
    assert_eq!(render_human(Reporter::Dot, &res), "x\n");
    let t = tally(&res);
    assert_eq!((t.passed, t.failed, t.errors, t.failed_files), (1, 0, 0, 1));
    assert_eq!(
        footer(1, &res, 1.0),
        "\n Test Files  1 failed (1)\n      Tests  1 passed (1)\n   Duration  1ms"
    );
    assert_eq!(exit_code(&t, true), 1);
    let j: serde_json::Value =
        serde_json::from_str(&render_json(&[file], &res, 0.0, false)).unwrap();
    assert_eq!(j["success"], false);
    assert_eq!(j["testResults"][0]["status"], "failed");
}

#[test]
fn files_are_named_relative_to_the_root() {
    // turbo-test#63 item 2: with `--root` the reports name files relative to it (vitest); a file
    // outside it, or no root, keeps the given path.
    let cwd = std::env::current_dir().unwrap();
    let files = vec![
        PathBuf::from("sub/a.test.ts"),
        cwd.join("sub/in/b.test.ts"),
        PathBuf::from("other/c.test.ts"),
    ];
    assert_eq!(
        display_names(&files, Some(Path::new("sub"))),
        [
            PathBuf::from("a.test.ts"),
            PathBuf::from("in/b.test.ts"),
            PathBuf::from("other/c.test.ts")
        ]
    );
    assert_eq!(display_names(&files, None), files);
    assert_eq!(
        display_names(&files[..1], Some(Path::new(""))),
        [PathBuf::from("sub/a.test.ts")]
    );
}

#[test]
fn junit_numbers_dates_and_escapes_are_vitests() {
    // (ms / 1000).toLocaleString('en-US', { useGrouping: false, maximumFractionDigits: 10 })
    assert_eq!(seconds(0.0), "0");
    assert_eq!(seconds(1.140542), "0.001140542");
    assert_eq!(seconds(12345.0), "12.345");
    assert_eq!(seconds(0.00000000001), "0");
    assert_eq!(seconds(2000.0), "2");
    // new Date(ms).toISOString()
    assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso_time(951_782_400_123), "2000-02-29T00:00:00.123Z");
    assert_eq!(iso_time(1_793_048_123_456), "2026-10-26T20:55:23.456Z");
    assert_eq!(iso_time(4_102_444_799_999), "2099-12-31T23:59:59.999Z");
    // the five entities, forbidden and discouraged characters left out
    assert_eq!(
        xml_esc("a&<>\"'\u{0}\u{8}\t\n\u{b}\u{1f}\u{7f}\u{85}\u{9f}\u{fdd0}\u{fffe}\u{1fffe}z"),
        "a&amp;&lt;&gt;&quot;&apos;\t\n\u{85}z"
    );
}

#[test]
fn junit_load_errors_without_an_error_object_name_their_kind() {
    let e = load_error_info("module errored: TypeError: x is not a function");
    assert_eq!(
        (e.name.as_str(), e.message.as_str()),
        ("TypeError", "x is not a function")
    );
    let e = load_error_info("No test suite found in file /p/a.test.ts");
    assert_eq!(
        (e.name.as_str(), e.message.as_str()),
        ("Error", "No test suite found in file /p/a.test.ts")
    );
    let e = load_error_info("graph load failed");
    assert_eq!(
        (e.name.as_str(), e.message.as_str()),
        ("Error", "graph load failed")
    );
}

/// A junit report with its run-dependent `timestamp` / `hostname` blanked.
fn junit_norm(x: &str) -> String {
    x.lines()
        .map(|l| match (l.find(" timestamp=\""), l.find(" tests=\"")) {
            (Some(a), Some(b)) if a < b => {
                format!("{} timestamp=\"S\" hostname=\"H\"{}", &l[..a], &l[b..])
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// turbo-test#164: the summary line names a load error as the module runner does
/// (`Error: boom`), without the ESM-graph loader's load-phase prefix; the error message kept
/// for the reporters is unchanged.
#[test]
fn the_error_line_drops_the_load_phase_prefix() {
    let f = Path::new("/p/a.test.ts");
    for (e, shown) in [
        ("module errored: Error: top boom", "Error: top boom"),
        ("evaluate threw: TypeError: x", "TypeError: x"),
        (
            "instantiate: SyntaxError: The requested module './l.mjs' does not provide an export named 'n'",
            "SyntaxError: The requested module './l.mjs' does not provide an export named 'n'",
        ),
        ("graph load failed", "graph load failed"),
        ("Error: module errored: inside", "Error: module errored: inside"),
    ] {
        let r = file_result(0, f, Err(e.into()), 1.0);
        assert_eq!(r.line, format!("ERROR /p/a.test.ts  ({shown})"));
        assert_eq!(r.error_msg, e);
    }
}
