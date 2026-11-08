//! Microtask interleaving between hooks and tests (turbo-test#251): vitest 4.1.12 awaits each
//! phase (beforeAll / beforeEach / test / afterEach / afterAll, onTestFinished, beforeEach and
//! beforeAll cleanups, retries, it.fails, nested and skipped describes) through a fixed async
//! chain, so a microtask chain queued in one phase settles at a fixed point of the next ones.
//! Each probe in `tests/data/microtask-order` logs that order; `<probe>.vitest.json` is the log
//! recorded from real vitest 4.1.12 + jsdom 27.4.0 (the same with `isolate: false`). `gap` logs,
//! at each phase start, how many microtask turns passed since the previous phase started; `shapes`
//! the same across describe / skipped-test boundaries, `edge` across async and throwing hooks,
//! ctx.skip, onTestFailed, thenables and it.each (its failing tests fail the run).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

const PROBES: [(&str, &str, &str); 5] = [
    (
        "order",
        include_str!("data/microtask-order/order.test.js"),
        include_str!("data/microtask-order/order.vitest.json"),
    ),
    (
        "deep",
        include_str!("data/microtask-order/deep.test.js"),
        include_str!("data/microtask-order/deep.vitest.json"),
    ),
    (
        "gap",
        include_str!("data/microtask-order/gap.test.js"),
        include_str!("data/microtask-order/gap.vitest.json"),
    ),
    (
        "shapes",
        include_str!("data/microtask-order/shapes.test.js"),
        include_str!("data/microtask-order/shapes.vitest.json"),
    ),
    (
        "edge",
        include_str!("data/microtask-order/edge.test.js"),
        include_str!("data/microtask-order/edge.vitest.json"),
    ),
];

/// Every `ORDER <probe> <json>` line of the run, as (probe, log).
fn orders(stdout: &str) -> Vec<(String, serde_json::Value)> {
    stdout
        .lines()
        .filter_map(|l| l.split("ORDER ").nth(1))
        .map(|rest| {
            let (name, json) = rest.split_once(' ').unwrap();
            (name.to_string(), serde_json::from_str(json).unwrap())
        })
        .collect()
}

/// Runs each probe `copies` times (one file per copy) under `config`; every file's log must
/// equal vitest's.
fn check(name: &str, config: &str, copies: usize) {
    let p = Project::new(name);
    p.file(
        "package.json",
        &format!(r#"{{"name":"{name}","type":"module"}}"#),
    );
    p.file("vitest.config.js", config);
    for (probe, src, _) in PROBES {
        for i in 0..copies {
            p.file(&format!("{probe}{i}.test.js"), src);
        }
    }
    let r = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "verbose"], &[]);
    let got = orders(&r.stdout);
    for (probe, _, want) in PROBES {
        let want: serde_json::Value = serde_json::from_str(want).unwrap();
        let logs: Vec<_> = got.iter().filter(|(n, _)| n == probe).collect();
        // a failing file (edge) runs again in a fresh isolate under reuse: it logs again
        assert!(logs.len() >= copies, "{probe}: {}", r.all());
        for (_, log) in logs {
            assert_eq!(
                log, &want,
                "{probe}: turbo-test's order differs from vitest's"
            );
        }
    }
}

#[test]
fn microtask_order_matches_vitest_in_fresh_isolates() {
    check(
        "rc-microtask-fresh",
        "export default { test: { environment: 'jsdom' } };\n",
        1,
    );
}

#[test]
fn microtask_order_matches_vitest_under_isolate_reuse() {
    check(
        "rc-microtask-reuse",
        "export default { test: { environment: 'jsdom', isolate: false } };\n",
        2,
    );
}
