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
    run_with(probe, &[])
}

/// [`run`] with extra CLI arguments.
fn run_with(probe: &Probe, args: &[&str]) -> (Value, Value, common::Run) {
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
            &[
                "--jobs",
                "1",
                "--reporter",
                "json",
                "--outputFile",
                "r.json",
            ],
            args,
        ]
        .concat(),
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

/// turbo-test#257: `{ repeats: n }` runs a test 1 + n times, each repeat with its hooks and
/// handlers and `result.repeatCount` counting them; retries count on across repeats; a suite's
/// `repeats` is inherited by its tests; a repeat that fails fails the test even when a later one
/// passes (an it.fails one passes).
#[test]
fn repeats_run_a_test_again_as_vitest() {
    let r = check(&Probe {
        name: "rep",
        src: include_str!("data/vitest-gaps/rep.test.js"),
        want: include_str!("data/vitest-gaps/rep.vitest.json"),
        config: JSDOM,
    });
    assert_eq!(r.code, Some(0), "{}", r.all());
    let r = check(&Probe {
        name: "repfail",
        src: include_str!("data/vitest-gaps/repfail.test.js"),
        want: include_str!("data/vitest-gaps/repfail.vitest.json"),
        config: JSDOM,
    });
    assert_eq!(r.code, Some(1), "{}", r.all());
}

/// vitest's verbose reporter marks a repeated test `(repeat xN)` (after `(retry xN)`), and a
/// test failing in a repeat with that repeat's error.
#[test]
fn repeats_show_in_the_verbose_reporter() {
    let p = Project::new("rc-gaps-rep-verbose");
    p.file("package.json", r#"{"name":"rv","type":"module"}"#);
    p.file("vitest.config.js", JSDOM);
    p.file("rep.test.js", include_str!("data/vitest-gaps/rep.test.js"));
    p.file(
        "repfail.test.js",
        include_str!("data/vitest-gaps/repfail.test.js"),
    );
    let r = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "verbose"],
        &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
    );
    let line = |name: &str| {
        r.stdout
            .lines()
            .find(|l| l.contains(&format!("> {name} ")))
            .unwrap_or_else(|| panic!("no line for {name}\n{}", r.all()))
            .to_string()
    };
    // as vitest 4.1.12 prints them (tests/data/vitest-gaps: rep.test.js, repfail.test.js)
    for (name, suffix) in [
        ("two", "ms (repeat x2)"),
        ("retry", "ms (retry x2) (repeat x2)"),
        ("zero", "ms"),
        ("in", "ms (repeat x1)"),
        ("own", "ms (repeat x2)"),
        ("fails-in-repeat-1", "ms (repeat x2)"),
        ("fails", "ms (repeat x1)"),
    ] {
        assert!(
            line(name).ends_with(suffix),
            "{name}: {}\n{}",
            line(name),
            r.all()
        );
    }
    assert!(r.stdout.contains("→ repeat 2"), "{}", r.all());
}

const HOOK_HINT: &str = "If this is a long-running hook, pass a timeout value as the last argument or configure it globally with \"hookTimeout\".";

/// turbo-test#255: a beforeAll / beforeEach / afterEach / afterAll (and a beforeEach's cleanup)
/// that runs longer than its timeout fails with vitest's error: the per-hook timeout argument,
/// else the config's `test.hookTimeout`. A beforeEach timing out fails its test (its afterEach
/// still runs); an afterEach one aborts the test's signal; a beforeAll one skips the suite's
/// tests and fails the suite, an afterAll one (or a beforeAll's cleanup) fails the suite. An
/// onTestFinished handler times out the same way.
#[test]
fn hooks_time_out_as_vitest() {
    let r = check(&Probe {
        name: "hook",
        src: include_str!("data/vitest-gaps/hook.test.js"),
        want: include_str!("data/vitest-gaps/hook.vitest.json"),
        config: "export default { test: { environment: 'jsdom', hookTimeout: 60 } };\n",
    });
    assert_eq!(r.code, Some(1), "{}", r.all());
    // the suites whose beforeAll / afterAll / beforeAll cleanup timed out fail with that error,
    // as vitest's Failed Suites section prints them
    let p = Project::new("rc-gaps-hook-suites");
    p.file("package.json", r#"{"name":"hs","type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'jsdom', hookTimeout: 60 } };\n",
    );
    p.file(
        "hook.test.js",
        include_str!("data/vitest-gaps/hook.test.js"),
    );
    let r = turbo_test(
        &p.dir,
        &["--reporter", "verbose"],
        &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
    );
    for (suite, ms) in [("ba", 20), ("aa", 25), ("ba-cleanup", 15)] {
        let want =
            format!(" FAIL  hook.test.js > {suite}\nError: Hook timed out in {ms}ms.\n{HOOK_HINT}");
        assert!(r.all().contains(&want), "{want}\n{}", r.all());
    }
}

/// vitest's default hookTimeout is 10 s (run on turbo-test's virtual clock: the test does not
/// wait for it); `--hookTimeout` sets it, over the config's.
#[test]
fn hook_timeout_defaults_to_ten_seconds_and_follows_the_flag() {
    let probe = Probe {
        name: "hookdef",
        src: include_str!("data/vitest-gaps/hookdef.test.js"),
        want: include_str!("data/vitest-gaps/hookdef.vitest.json"),
        config: JSDOM,
    };
    check(&probe);
    let (order, _, r) = run_with(
        &Probe {
            config: "export default { test: { environment: 'jsdom', hookTimeout: 30000 } };\n",
            ..probe
        },
        &["--hookTimeout", "50"],
    );
    assert_eq!(
        order,
        serde_json::json!([format!("fail Hook timed out in 50ms.\n{HOOK_HINT}")]),
        "{}",
        r.all()
    );
}

/// turbo-test#268: `--hookTimeout 0` disables the hook timeout entirely, as vitest's
/// `withTimeout` (`timeout <= 0 || Infinity` → no timeout). Before the fix, turbo-test fell back
/// to the 10 000 ms default when the env value was `0`, so a hook that waited more than 10 s
/// timed out where vitest's would not. Probe: a beforeEach that waits 10 200 ms with
/// `--hookTimeout 0` passes under both runners.
#[test]
fn hook_timeout_zero_disables_the_timeout_as_vitest() {
    let probe = Probe {
        name: "hookzero",
        src: include_str!("data/vitest-gaps/hookzero.test.js"),
        want: include_str!("data/vitest-gaps/hookzero.vitest.json"),
        config: JSDOM,
    };
    let want: Value = serde_json::from_str(probe.want).unwrap();
    let (order, tests, r) = run_with(&probe, &["--hookTimeout", "0"]);
    assert_eq!(order, want["order"], "hookzero order: {}", r.all());
    assert_eq!(tests, want["tests"], "hookzero tests: {}", r.all());
    assert_eq!(r.code, Some(0), "hookzero exit: {}", r.all());
    assert!(
        !r.all().contains("Hook timed out"),
        "no timeout error expected: {}",
        r.all()
    );
}

/// turbo-test#256: concurrent tests run together, as vitest's runSuite: a suite's consecutive
/// concurrent children (it.concurrent / test.concurrent, the tests and nested suites of a
/// describe.concurrent, test.sequential / describe.sequential opting out) start together, at
/// most `maxConcurrency` (5) at a time, and the next group waits for them; hooks run around each
/// test. The JSON report keeps declaration order.
#[test]
fn concurrent_tests_run_together_as_vitest() {
    let r = check(&Probe {
        name: "conc",
        src: include_str!("data/vitest-gaps/conc.test.js"),
        want: include_str!("data/vitest-gaps/conc.vitest.json"),
        config: JSDOM,
    });
    assert_eq!(r.code, Some(0), "{}", r.all());
}

/// The config's `test.maxConcurrency` limits them, and `test.sequence.concurrent` makes every
/// test and describe concurrent (test.sequential / describe.sequential still opt out).
#[test]
fn max_concurrency_and_sequence_concurrent_come_from_the_config() {
    check(&Probe {
        name: "mc",
        src: include_str!("data/vitest-gaps/mc.test.js"),
        want: include_str!("data/vitest-gaps/mc.vitest.json"),
        config: include_str!("data/vitest-gaps/mc.config.js"),
    });
    check(&Probe {
        name: "seqconc",
        src: include_str!("data/vitest-gaps/seqconc.test.js"),
        want: include_str!("data/vitest-gaps/seqconc.vitest.json"),
        config: include_str!("data/vitest-gaps/seq.config.js"),
    });
}

/// `--maxConcurrency` / `--sequence.concurrent` do the same as the config's (inline value form
/// too), the flags winning over it.
#[test]
fn max_concurrency_and_sequence_concurrent_flags() {
    for (probe, src, want, args) in [
        (
            "mc",
            include_str!("data/vitest-gaps/mc.test.js"),
            include_str!("data/vitest-gaps/mc.vitest.json"),
            &["--maxConcurrency", "2"][..],
        ),
        (
            "seqconc",
            include_str!("data/vitest-gaps/seqconc.test.js"),
            include_str!("data/vitest-gaps/seqconc.vitest.json"),
            &["--sequence.concurrent", "--maxConcurrency=3"][..],
        ),
    ] {
        let want: Value = serde_json::from_str(want).unwrap();
        let (order, tests, r) = run_with(
            &Probe {
                name: probe,
                src,
                want: "",
                config: "export default { test: { environment: 'jsdom', maxConcurrency: 9 } };\n",
            },
            args,
        );
        assert_eq!(order, want["order"], "{probe}\n{}", r.all());
        assert_eq!(tests, want["tests"], "{probe}\n{}", r.all());
    }
}
