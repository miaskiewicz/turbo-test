//! Jest 30's @jest/expect-utils `equals` compares two Error instances by message alone
//! (`if (a instanceof Error && b instanceof Error) return a.message === b.message`). Name,
//! cause, own enumerable props and exact subclass do NOT matter — the real-world witness is
//! `expect(err).toEqual(new BadRequestException(msg))` in entity-tax-registration.spec
//! (flux-apis), where the thrown exception carries extra enumerable properties. turbo-test's
//! Error equality walks the shape (className / name / message), the cause and own enumerable
//! properties, so the same assertion fails (turbo-test#443). Match Jest in jest projects;
//! vitest projects keep their stricter rule. Oracle: real Jest 30 `--json`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn statuses(p: &Project) -> Vec<(String, String)> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    report["testResults"][0]["assertionResults"]
        .as_array()
        .unwrap_or_else(|| panic!("no tests: {}", run.all()))
        .iter()
        .map(|a| {
            (
                a["fullName"].as_str().unwrap().to_string(),
                a["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

const CLASS_HEADER: &str = r#"
class BadRequestException extends Error {
  constructor(message, extra) {
    super(message);
    this.name = 'BadRequestException';
    this.status = 400;
    if (extra) Object.assign(this, extra);
  }
}
class BadRequestException2 extends Error {
  constructor(message) { super(message); this.name = 'BadRequestException'; }
}
"#;

#[test]
fn jest_project_error_toequal_matches_jest_oracle() {
    let p = Project::new("jest-error-toequal-443");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    let body = format!(
        "{CLASS_HEADER}\n{}",
        r#"
test('same-class same-message', () => {
  expect(new BadRequestException('nope')).toEqual(new BadRequestException('nope'));
});
test('same-name different-class', () => {
  expect(new BadRequestException('nope')).toEqual(new BadRequestException2('nope'));
});
test('different-name same-message', () => {
  const a = new Error('x'); a.name = 'TypeError';
  const b = new Error('x'); b.name = 'RangeError';
  expect(a).toEqual(b);
});
test('extra-props-on-actual-only', () => {
  // the real-world witness: the thrown error carries extra enumerable properties
  const got = new BadRequestException('nope', { response: { message: 'nope' }, cause: new Error('inner') });
  expect(got).toEqual(new BadRequestException('nope'));
});
test('extra-props-on-expected-only', () => {
  const got = new BadRequestException('nope');
  expect(got).toEqual(new BadRequestException('nope', { context: { ref: 42 } }));
});
test('cause-differs-still-passes', () => {
  expect(new Error('x', { cause: new Error('a') })).toEqual(new Error('x', { cause: new Error('b') }));
});
test('different-message-fails', () => {
  expect(new BadRequestException('a')).toEqual(new BadRequestException('b'));
});
test('error-vs-plain-object-fails', () => {
  expect(new Error('x')).toEqual({ message: 'x' });
});
"#
    );
    p.file("err.test.js", &body);
    let got = statuses(&p);
    let expect = [
        ("same-class same-message", "passed"),
        ("same-name different-class", "passed"),
        ("different-name same-message", "passed"),
        ("extra-props-on-actual-only", "passed"),
        ("extra-props-on-expected-only", "passed"),
        ("cause-differs-still-passes", "passed"),
        ("different-message-fails", "failed"),
        ("error-vs-plain-object-fails", "failed"),
    ];
    assert_eq!(got.len(), expect.len(), "{got:?}");
    for (i, (name, want)) in expect.iter().enumerate() {
        assert_eq!(got[i].0, *name, "{got:?}");
        assert_eq!(got[i].1, *want, "{name}: {got:?}");
    }
}

#[test]
fn vitest_project_error_toequal_keeps_strict_vitest_behaviour() {
    // vitest fails when own enumerable properties differ between the errors
    let p = Project::new("vitest-error-toequal-443");
    p.with_esbuild();
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    let body = format!(
        "import {{ test, expect }} from 'vitest';\n{CLASS_HEADER}\n{}",
        r#"
test('extras on actual fails under vitest', () => {
  const got = new BadRequestException('nope', { response: { message: 'nope' } });
  expect(got).toEqual(new BadRequestException('nope'));
});
test('equal plain Errors still pass', () => {
  expect(new Error('x')).toEqual(new Error('x'));
});
"#
    );
    p.file("err.test.js", &body);
    let got = statuses(&p);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        got[0],
        (
            "extras on actual fails under vitest".into(),
            "failed".into()
        )
    );
    assert_eq!(
        got[1],
        ("equal plain Errors still pass".into(), "passed".into())
    );
}
