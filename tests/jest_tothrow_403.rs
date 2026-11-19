//! turbo-test#403 follow-up: `toThrow(errorInstance)` follows the project's framework. In a jest
//! project it is real Jest 30's rule (message, then cause, then the class for a custom error
//! class), as the Flux specs expect of `rejects.toThrow(new BadRequestException(msg))` on a coded
//! exception; in a vitest project it stays vitest's (equality of name, message, cause and own
//! props). The expected outcomes are real Jest 30.5.2's and real vitest 5.1.0's on the same file.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const CASES: &str = r#"
class HttpError extends Error {
  constructor(body) {
    super(typeof body === 'string' ? body : body.message);
    this.name = 'HttpError';
    this.response = body;
    this.cause = undefined;
  }
}
class OtherError extends Error {}
const thrower = (e) => () => { throw e; };

test('t01 same class and message, different own props', () => {
  expect(thrower(new HttpError({ message: 'm', params: { d: ['A'] } }))).toThrow(new HttpError('m'));
});
test('t02 same class, different message', () => {
  expect(thrower(new HttpError('m'))).toThrow(new HttpError('n'));
});
test('t03 custom expected class, other thrown class, same message', () => {
  expect(thrower(new OtherError('m'))).toThrow(new HttpError('m'));
});
test('t04 plain Error expected, subclass thrown, same message', () => {
  expect(thrower(new TypeError('m'))).toThrow(new Error('m'));
});
test('t05 same message, different causes', () => {
  expect(thrower(new Error('m', { cause: 'a' }))).toThrow(new Error('m', { cause: 'b' }));
});
test('t06 same message, same causes', () => {
  expect(thrower(new Error('m', { cause: { k: 1 } }))).toThrow(new Error('m', { cause: { k: 1 } }));
});
test('t07 thrown has a cause, expected has none', () => {
  expect(thrower(new Error('m', { cause: 'a' }))).toThrow(new Error('m'));
});
test('t08 plain object with the message', () => {
  expect(thrower(new Error('m'))).toThrow({ message: 'm' });
});
test('t09 rejects: same class and message, different own props', async () => {
  await expect(Promise.reject(new HttpError({ message: 'm', code: 'X' }))).rejects.toThrow(new HttpError('m'));
});
test('t10 not: a different message', () => {
  expect(thrower(new HttpError('m'))).not.toThrow(new HttpError('n'));
});
test('t11 not: the same message, different own props', () => {
  expect(thrower(new HttpError({ message: 'm', code: 'X' }))).not.toThrow(new HttpError('m'));
});
test('t12 a thrown string against an error with that message', () => {
  expect(thrower('m')).toThrow(new Error('m'));
});
test('t13 a thrown object with a message against an error', () => {
  expect(thrower({ message: 'm' })).toThrow(new Error('m'));
});
test('t14 asymmetric matcher', () => {
  expect(thrower(new HttpError('m'))).toThrow(expect.objectContaining({ message: 'm' }));
});
test('t15 did not throw', () => {
  expect(() => {}).toThrow(new Error('m'));
});
test('t16 not: did not throw', () => {
  expect(() => {}).not.toThrow(new Error('m'));
});
test('t17 rejects: different message', async () => {
  await expect(Promise.reject(new HttpError('m'))).rejects.toThrow(new HttpError('n'));
});
"#;

const JEST: &[(&str, &str)] = &[
    ("t01", "passed"),
    ("t02", "failed"),
    ("t03", "failed"),
    ("t04", "passed"),
    ("t05", "failed"),
    ("t06", "passed"),
    ("t07", "failed"),
    ("t08", "passed"),
    ("t09", "passed"),
    ("t10", "passed"),
    ("t11", "failed"),
    ("t12", "failed"),
    ("t13", "passed"),
    ("t14", "passed"),
    ("t15", "failed"),
    ("t16", "passed"),
    ("t17", "failed"),
];
const VITEST: &[(&str, &str)] = &[
    ("t01", "failed"),
    ("t02", "failed"),
    ("t03", "failed"),
    ("t04", "failed"),
    ("t05", "failed"),
    ("t06", "passed"),
    ("t07", "passed"),
    ("t08", "failed"),
    ("t09", "failed"),
    ("t10", "passed"),
    ("t11", "passed"),
    ("t12", "failed"),
    ("t13", "failed"),
    ("t14", "passed"),
    ("t15", "failed"),
    ("t16", "passed"),
    ("t17", "failed"),
];

/// Each test's (id, status) from the JSON report of `cases.test.js` run in `p`.
fn outcomes(p: &Project) -> Vec<(String, String)> {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    let mut got: Vec<(String, String)> = report["testResults"][0]["assertionResults"]
        .as_array()
        .unwrap_or_else(|| panic!("{}", run.all()))
        .iter()
        .map(|a| {
            let title = a["title"].as_str().unwrap();
            (
                title[..3].to_string(),
                a["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    got.sort();
    got
}

fn want(table: &[(&str, &str)]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(k, s)| (k.to_string(), s.to_string()))
        .collect()
}

#[test]
fn to_throw_an_error_instance_follows_jest_in_a_jest_project() {
    let p = Project::new("jest-tothrow-403-jest");
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p.file("cases.test.js", CASES);
    assert_eq!(outcomes(&p), want(JEST));
}

#[test]
fn to_throw_an_error_instance_follows_vitest_in_a_vitest_project() {
    let p = Project::new("jest-tothrow-403-vitest");
    p.file("package.json", "{}");
    p.file(
        "vitest.config.mjs",
        "export default { test: { globals: true } };",
    );
    p.file("cases.test.js", CASES);
    assert_eq!(outcomes(&p), want(VITEST));
}
