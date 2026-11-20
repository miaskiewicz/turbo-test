//! Node v26.11.0 orderings and stream options: nextTicks before promise jobs after every host
//! callback (sockets, child pipes, HTTP, MessagePort, timers, immediates), node:stream Writable's
//! decodeStrings / default encoding / _construct / `writable`, and a failing node:assert diff that
//! is reported instead of exhausting the heap. The data files also run under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn node_project(name: &str, file: &str, source: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(file, source);
    p
}

fn assert_all_pass(name: &str, source: &str, count: u64) {
    let p = node_project(name, "order.test.cjs", source);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn host_callbacks_run_their_next_ticks_then_promise_jobs_like_node() {
    assert_all_pass(
        "host-callback-order-085",
        include_str!("data/compat-085/host-callback-order.cjs"),
        6,
    );
}

#[test]
fn writable_options_construct_and_writable_state_match_node() {
    assert_all_pass(
        "writable-options-085",
        include_str!("data/compat-085/writable-options.cjs"),
        7,
    );
}

#[test]
fn a_failing_node_assert_deep_equal_reports_its_diff() {
    let p = node_project(
        "assert-diff-085",
        "assert.test.cjs",
        include_str!("data/compat-085/assert-diff.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let all = run.all();
    assert_eq!(report["numPassedTests"], 1, "{all}");
    assert_eq!(report["numFailedTests"], 1, "{all}");
    let failure = report["testResults"][0]["assertionResults"][0]["failureMessages"][0]
        .as_str()
        .unwrap_or_default();
    assert!(
        failure.contains("Expected values to be strictly deep-equal"),
        "{all}"
    );
    assert!(!all.contains("heap out of memory"), "{all}");
}
