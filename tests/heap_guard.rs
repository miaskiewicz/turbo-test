//! The per-isolate heap guard (src/heap_guard.rs): `TURBO_MAX_HEAP_MB` caps each isolate's heap
//! and a file that runs into it fails with the guard's error while the run goes on (fresh and
//! reused isolates); `TURBO_HEAP_LOG` prints one `[heap]` line per file; under isolate reuse a
//! worker whose live heap passes `TURBO_HEAP_RECYCLE_MB` gets a new isolate for its next file.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// A file that keeps every array it allocates, and two that pass and log how many files their
/// isolate has run (`SEEN=<n>`).
fn heap_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "a-oom.test.js",
        "test('grows', () => { const keep = []; for (;;) keep.push(new Array(100000).fill(keep.length)); });\n",
    );
    let ok = "globalThis.__seen = (globalThis.__seen || 0) + 1;\n\
              test('ok', () => { console.log('SEEN=' + globalThis.__seen); expect(1).toBe(1); });\n";
    p.file("b-ok.test.js", ok);
    p.file("c-ok.test.js", ok);
    p
}

/// The `--reporter json` message of each file, by file name.
fn messages(stdout: &str) -> Vec<(String, String)> {
    let report = json_report(&common::Run {
        code: Some(0),
        stdout: stdout.to_string(),
        stderr: String::new(),
    });
    let mut v: Vec<(String, String)> = report["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap().rsplit('/').next().unwrap().to_string(),
                f["message"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

/// The `[heap]` lines of a run's stderr as (limit=…, recycled=…, file name), in order.
fn heap_lines(stderr: &str) -> Vec<(String, String, String)> {
    stderr
        .lines()
        .filter(|l| l.starts_with("[heap] ThreadId("))
        .map(|l| {
            let field = |k: &str| {
                l.split_whitespace()
                    .find_map(|w| w.strip_prefix(k))
                    .unwrap()
                    .to_string()
            };
            for k in ["used=", "total="] {
                assert!(field(k).ends_with("MB"), "{l}");
            }
            (
                field("limit="),
                field("recycled="),
                l.rsplit('/').next().unwrap().to_string(),
            )
        })
        .collect()
}

const OOM_64: &str =
    "JavaScript heap out of memory (limit 64 MB): the file was aborted and its isolate discarded";

#[test]
fn a_file_past_the_heap_limit_fails_and_the_run_goes_on_with_a_fresh_isolate() {
    let p = heap_project("hg-fresh");
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[
            ("TURBO_JOBS", "1"),
            ("TURBO_MAX_HEAP_MB", "64"),
            ("TURBO_HEAP_LOG", "1"),
        ],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        messages(&r.stdout),
        [
            ("a-oom.test.js".to_string(), OOM_64.to_string()),
            ("b-ok.test.js".to_string(), String::new()),
            ("c-ok.test.js".to_string(), String::new()),
        ],
        "{}",
        r.all()
    );
    // one line per file that ran to its end on a 64 MB isolate; a fresh isolate per file
    let lines = heap_lines(&r.stderr);
    for f in ["b-ok.test.js", "c-ok.test.js"] {
        assert!(
            lines.contains(&("64MB".to_string(), "false".to_string(), f.to_string())),
            "{f}: {lines:?}\n{}",
            r.all()
        );
    }
    // the files' console output, wherever the json reporter puts it
    assert_eq!(r.all().matches("SEEN=1").count(), 2, "{}", r.all());
}

#[test]
fn without_the_log_switch_there_are_no_heap_lines() {
    let p = heap_project("hg-nolog");
    for log in ["", "0"] {
        let r = turbo_test(
            &p.dir,
            &["b-ok.test.js"],
            &[("TURBO_JOBS", "1"), ("TURBO_HEAP_LOG", log)],
        );
        assert_eq!(r.code, Some(0), "{}", r.all());
        assert!(!r.stderr.contains("[heap]"), "TURBO_HEAP_LOG={log:?}\n{}", r.all());
    }
}

#[test]
fn a_reused_isolate_past_the_heap_limit_fails_its_file_and_is_replaced() {
    let p = heap_project("hg-reuse-oom");
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[
            ("TURBO_JOBS", "1"),
            ("TURBO_REUSE_ISOLATE", "1"),
            ("TURBO_MAX_HEAP_MB", "64"),
        ],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        messages(&r.stdout),
        [
            ("a-oom.test.js".to_string(), OOM_64.to_string()),
            ("b-ok.test.js".to_string(), String::new()),
            ("c-ok.test.js".to_string(), String::new()),
        ],
        "{}",
        r.all()
    );
}

#[test]
fn a_reused_isolate_is_kept_below_the_recycle_threshold_and_replaced_above_it() {
    let p = heap_project("hg-recycle");
    let files = ["b-ok.test.js", "c-ok.test.js"];
    // the default threshold (60% of the limit): the second file runs on the first one's isolate
    let r = turbo_test(
        &p.dir,
        &files,
        &[
            ("TURBO_JOBS", "1"),
            ("TURBO_REUSE_ISOLATE", "1"),
            ("TURBO_HEAP_LOG", "1"),
            ("TURBO_MAX_HEAP_MB", "64"),
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("SEEN=1") && r.stdout.contains("SEEN=2"),
        "{}",
        r.all()
    );
    let kept: Vec<_> = heap_lines(&r.stderr)
        .into_iter()
        .map(|(limit, recycled, _)| (limit, recycled))
        .collect();
    assert_eq!(
        kept,
        [
            ("64MB".to_string(), "false".to_string()),
            ("64MB".to_string(), "false".to_string())
        ],
        "{}",
        r.all()
    );
    // a 0 MB threshold: every file's isolate is replaced after it, so each starts afresh
    let r = turbo_test(
        &p.dir,
        &files,
        &[
            ("TURBO_JOBS", "1"),
            ("TURBO_REUSE_ISOLATE", "1"),
            ("TURBO_HEAP_LOG", "1"),
            ("TURBO_HEAP_RECYCLE_MB", "0"),
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(r.stdout.matches("SEEN=1").count(), 2, "{}", r.all());
    let recycled: Vec<_> = heap_lines(&r.stderr)
        .into_iter()
        .map(|(_, recycled, _)| recycled)
        .collect();
    assert_eq!(recycled, ["true", "true"], "{}", r.all());
}
