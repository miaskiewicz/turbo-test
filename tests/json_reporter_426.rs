//! The JSON reporter keeps a timed-out test's message and a file-level hook error
//! (turbo-test#426). Oracle: vitest 5.1.0's JSON reporter on these files: a timeout's
//! failureMessages entry is the error's stack headed by `Error: Test timed out in 50ms.` (vitest 5's
//! makeTimeoutError puts its message into the registration stack); a file's `message` is the first
//! error of the file task (its top-level beforeAll / afterAll hooks), and a nested suite's hook
//! error stays on that suite (the file message is empty).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const TIMEOUT_HINT: &str = "If this is a long-running test, pass a timeout value as the last argument or configure it globally with \"testTimeout\".";
const HOOK_HINT: &str = "If this is a long-running hook, pass a timeout value as the last argument or configure it globally with \"hookTimeout\".";

fn file_result<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"].as_str().unwrap().ends_with(name))
        .unwrap_or_else(|| panic!("no {name} in {report}"))
}

#[test]
fn json_reporter_keeps_timeout_text_and_file_hook_errors() {
    let p = Project::new("json-reporter-426");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "t.test.js",
        r#"import { it, describe, afterAll, expect } from 'vitest';
it('times out', async () => { await new Promise(() => {}); }, 50);
it('ok', () => { expect(1).toBe(1); });
describe('s', () => {
  afterAll(() => { throw new Error('after all boom'); });
  it('inner ok', () => {});
});
"#,
    );
    p.file(
        "u.test.js",
        r#"import { it, afterAll } from 'vitest';
afterAll(() => { throw new Error('root after all boom'); });
it('only ok', () => {});
"#,
    );
    p.file(
        "w.test.js",
        r#"import { it, beforeAll, afterAll } from 'vitest';
beforeAll(() => { throw 'a thrown string'; });
afterAll(() => { throw new TypeError('second error'); });
it('skipped by the failed beforeAll', () => {});
"#,
    );
    p.file(
        "x.test.js",
        r#"import { it, beforeAll, describe } from 'vitest';
beforeAll(async () => { await new Promise(() => {}); }, 30);
describe('d', () => { it('never runs', () => {}); });
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let t = file_result(&report, "t.test.js");
    let timeout = t["assertionResults"][0]["failureMessages"][0]
        .as_str()
        .unwrap();
    assert!(
        timeout.starts_with(&format!(
            "Error: Test timed out in 50ms.\n{TIMEOUT_HINT}\n    at "
        )),
        "{timeout}"
    );
    assert_eq!(t["status"], "failed");
    assert_eq!(t["message"], "");
    let messages: Vec<(&str, &str)> = ["u.test.js", "w.test.js", "x.test.js"]
        .iter()
        .map(|n| {
            let r = file_result(&report, n);
            (
                r["status"].as_str().unwrap(),
                r["message"].as_str().unwrap(),
            )
        })
        .collect();
    let hook = format!("Hook timed out in 30ms.\n{HOOK_HINT}");
    assert_eq!(
        messages,
        vec![
            ("failed", "root after all boom"),
            ("failed", "a thrown string"),
            ("failed", hook.as_str()),
        ],
        "{}",
        run.all()
    );
    assert_eq!(report["numFailedTests"], 1, "{}", run.all());
    assert_eq!(report["numPendingTests"], 2, "{}", run.all());
}
