//! Jest 30's --json emits a file-level `message` that carries every failed test's failure in a
//! bulleted "  ● <ancestors › ...> <title>\n\n<indented content>" shape, joined by "\n". turbo-test
//! emits vitest-shape in both project kinds (turbo-test#441); match Jest's shape in jest projects
//! (stripped of ANSI). Oracle: real Jest 30 `--json` on the same fixture.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn jest_project_file_message_is_jest_bulleted() {
    let p = Project::new("jest-reporter-message-441");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file(
        "fails.test.js",
        r#"
describe('outer', () => {
  describe('inner', () => {
    test('a', () => { expect(1).toBe(2); });
  });
  test('sibling', () => { throw new Error('sibling boom'); });
});
test('top', () => { expect(1).toBe(1); });
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let msg = report["testResults"][0]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("\u{25CF} outer \u{203A} inner \u{203A} a"),
        "no inner bullet: {msg:?}\n{}",
        run.all()
    );
    assert!(
        msg.contains("\u{25CF} outer \u{203A} sibling"),
        "no sibling bullet: {msg:?}\n{}",
        run.all()
    );
    // the bulleted title is indented by two spaces (Jest's TITLE_INDENT)
    assert!(
        msg.contains("  \u{25CF} outer \u{203A} inner \u{203A} a"),
        "wrong indent: {msg:?}"
    );
    // each failure's content follows the title with a blank line and lines indented by 4 spaces
    assert!(msg.contains("sibling boom"), "no content: {msg:?}");
    // multiple failures joined by \n (two bullets present)
    assert!(
        msg.matches("\u{25CF} ").count() >= 2,
        "expected >=2 bullets: {msg:?}"
    );
}

#[test]
fn jest_project_file_message_empty_when_all_pass() {
    let p = Project::new("jest-reporter-message-441-pass");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("ok.test.js", "test('ok', () => { expect(1).toBe(1); });\n");
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(
        report["testResults"][0]["message"].as_str().unwrap_or(""),
        ""
    );
}

#[test]
fn vitest_project_file_message_stays_vitest_shape() {
    // the vitest-shape message is empty for a plain failing test (only the root suite's hook
    // errors land there); make sure we have NOT started emitting Jest bullets in vitest projects.
    let p = Project::new("vitest-reporter-message-441");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file(
        "fails.test.js",
        r#"
import { test, expect } from 'vitest';
test('a', () => { expect(1).toBe(2); });
"#,
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let msg = report["testResults"][0]["message"].as_str().unwrap_or("");
    assert!(
        !msg.contains("\u{25CF}"),
        "vitest project should not emit the Jest bullet: {msg:?}"
    );
}
