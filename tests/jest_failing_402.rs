//! Jest's `test.failing` / `it.failing` and `.failing.each` (turbo-test#402), against real
//! Jest 30.5.2 (jest-circus `_callCircusTest`): only the test body is inverted — a body that
//! throws, rejects or times out passes, one that completes fails with Jest's message — while
//! beforeEach / afterEach errors and `expect.assertions` stay failures.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const PASSED_MSG: &str =
    "Failing test passed even though it was supposed to fail. Remove `.failing` to remove error.";

fn jest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p
}

/// Each test of the one file run: (full name, status, its first failure message).
fn outcomes(p: &Project) -> Vec<(String, String, String)> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let file = &report["testResults"][0];
    let tests = file["assertionResults"]
        .as_array()
        .unwrap_or_else(|| panic!("no tests: {}", run.all()));
    tests
        .iter()
        .map(|a| {
            let msg = a["failureMessages"][0].as_str().unwrap_or("");
            (
                a["fullName"].as_str().unwrap().to_string(),
                a["status"].as_str().unwrap().to_string(),
                msg.to_string(),
            )
        })
        .collect()
}

fn assert_outcome(got: &(String, String, String), name: &str, status: &str, msg: &str) {
    assert_eq!((got.0.as_str(), got.1.as_str()), (name, status), "{got:?}");
    assert!(got.2.contains(msg), "{name}: {:?} lacks {msg:?}", got.2);
}

#[test]
fn failing_inverts_the_body_and_its_each_tables() {
    let p = jest_project("jest-failing-402-body");
    p.file(
        "body.test.js",
        r#"
test.failing('body throws', () => { expect(1).toBe(2); });
test.failing('body passes', () => { expect(1).toBe(1); });
it.failing('async rejection', async () => { await Promise.resolve(); throw new Error('boom'); });
it.failing.each([[1, 2], [3, 4]])('row %i plus %i', (a, b) => { expect(a + b).toBe(0); });
it.failing.each`
  a    | b
  ${1} | ${1}
`('tagged $a $b', ({ a, b }) => { expect(a).toBe(b); });
test('control', () => { expect(1).toBe(1); });
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 7, "{got:?}");
    assert_outcome(&got[0], "body throws", "passed", "");
    assert_outcome(&got[1], "body passes", "failed", PASSED_MSG);
    assert_outcome(&got[2], "async rejection", "passed", "");
    assert_outcome(&got[3], "row 1 plus 2", "passed", "");
    assert_outcome(&got[4], "row 3 plus 4", "passed", "");
    assert_outcome(&got[5], "tagged 1 1", "failed", PASSED_MSG);
    assert_outcome(&got[6], "control", "passed", "");
}

#[test]
fn failing_keeps_hook_errors_as_failures() {
    let p = jest_project("jest-failing-402-hooks");
    p.file(
        "hooks.test.js",
        r#"
describe('be', () => {
  beforeEach(() => { throw new Error('be boom'); });
  test.failing('be error', () => { throw new Error('body'); });
});
describe('ae', () => {
  afterEach(() => { throw new Error('ae boom'); });
  test.failing('ae error', () => { throw new Error('body'); });
});
test('control', () => {});
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_outcome(&got[0], "be be error", "failed", "be boom");
    assert_outcome(&got[1], "ae ae error", "failed", "ae boom");
    // the body's own (expected) error is not reported next to the hook's
    assert!(!got[1].2.contains("body"), "{got:?}");
    assert_outcome(&got[2], "control", "passed", "");
}

#[test]
fn failing_timeouts_pass_and_assertion_counts_still_fail() {
    let p = jest_project("jest-failing-402-misc");
    p.file(
        "misc.test.js",
        r#"
test.failing('timeout', () => new Promise(() => {}), 50);
test.failing('assertions count', () => { expect.assertions(5); throw new Error('x'); });
test.concurrent.failing('concurrent', async () => { throw new Error('c'); });
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_outcome(&got[0], "timeout", "passed", "");
    assert_outcome(&got[1], "assertions count", "failed", "assertions");
    assert_outcome(&got[2], "concurrent", "passed", "");
}

// Not a Jest API: turbo-test's test context `skip()` (vitest's) is a skip, never the expected
// failure a `.failing` body is waiting for.
#[test]
fn failing_context_skip_still_skips() {
    let p = jest_project("jest-failing-402-ctx-skip");
    p.file(
        "skip.test.js",
        "test.failing('ctx skip', (ctx) => { ctx.skip(); throw new Error('unreached'); });\n",
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_outcome(&got[0], "ctx skip", "skipped", "");
}

#[test]
fn only_and_skip_failing_follow_their_modes() {
    let p = jest_project("jest-failing-402-modes");
    p.file(
        "modes.test.js",
        r#"
test.skip.failing('skipped', () => {});
test.only.failing('only one', () => { throw new Error('x'); });
it.only.failing.each([[1]])('only row %i', () => { throw new Error('y'); });
test('not run', () => {});
"#,
    );
    let got = outcomes(&p);
    assert_eq!(got.len(), 4, "{got:?}");
    assert_outcome(&got[0], "skipped", "skipped", "");
    assert_outcome(&got[1], "only one", "passed", "");
    assert_outcome(&got[2], "only row 1", "passed", "");
    assert_outcome(&got[3], "not run", "skipped", "");
}
