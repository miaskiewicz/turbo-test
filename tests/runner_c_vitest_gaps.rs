//! vitest-parity gaps found in the 0.8.0 review (turbo-test#255..#258). Each probe in
//! `tests/data/vitest-gaps` logs what ran, in which order, as `ORDER <probe> <json>`;
//! `<probe>.vitest.json` holds that log (`order`) and every test's `[fullName, status]` (`tests`)
//! recorded from real vitest 4.1.12 + jsdom 27.4.0 (`--reporter json`, the config named per
//! probe). turbo-test must log the same and report the same statuses.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};
use serde_json::Value;

/// A probe: its name, source, vitest's record and the vitest config it was recorded under.
struct Probe {
    name: &'static str,
    src: &'static str,
    want: &'static str,
    config: &'static str,
}

const JSDOM: &str = "export default { test: { environment: 'jsdom' } };\n";

/// Runs `probe` (in its own project) and returns (its ORDER log, its tests' [fullName, status],
/// the run).
fn run(probe: &Probe) -> (Value, Value, common::Run) {
    let p = Project::new(&format!("rc-gaps-{}", probe.name));
    p.file(
        "package.json",
        &format!(r#"{{"name":"{}","type":"module"}}"#, probe.name),
    );
    p.file("vitest.config.js", probe.config);
    p.file(&format!("{}.test.js", probe.name), probe.src);
    let r = turbo_test(
        &p.dir,
        &[
            "--jobs",
            "1",
            "--reporter",
            "json",
            "--outputFile",
            "r.json",
        ],
        &[],
    );
    let tag = format!("ORDER {} ", probe.name);
    let order = r
        .stdout
        .lines()
        .chain(r.stderr.lines())
        .find_map(|l| l.split(tag.as_str()).nth(1))
        .map(|j| serde_json::from_str(j).unwrap())
        .unwrap_or(Value::Null);
    let report: Value = std::fs::read_to_string(p.path("r.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let tests = report["testResults"][0]["assertionResults"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| serde_json::json!([t["fullName"], t["status"]]))
                .collect()
        })
        .unwrap_or_default();
    (order, Value::Array(tests), r)
}

/// The probe's log and statuses equal vitest's; returns the run for further checks.
fn check(probe: &Probe) -> common::Run {
    let want: Value = serde_json::from_str(probe.want).unwrap();
    let (order, tests, r) = run(probe);
    assert_eq!(
        order,
        want["order"],
        "{}: order differs from vitest's\n{}",
        probe.name,
        r.all()
    );
    assert_eq!(
        tests,
        want["tests"],
        "{}: statuses differ from vitest's\n{}",
        probe.name,
        r.all()
    );
    r
}

/// turbo-test#258: `onTestFinished` / `onTestFailed` imported from 'vitest' register on the
/// running test (from its body, a beforeEach or an afterEach), run last registered first with
/// the test's context, `onTestFailed` only for a failed attempt; outside a test they throw
/// vitest's error, and a non-function is a TypeError.
#[test]
fn on_test_finished_and_failed_import_from_vitest() {
    check(&Probe {
        name: "on",
        src: include_str!("data/vitest-gaps/on.test.js"),
        want: include_str!("data/vitest-gaps/on.vitest.json"),
        config: JSDOM,
    });
}
