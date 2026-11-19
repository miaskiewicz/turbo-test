//! `.each` titles and arguments follow jest-each in a Jest project and vitest in a vitest one
//! (0.8.5). The fixtures under `data/jest-each-085/` cover every format verb (`%s %d %i %f %p
//! %j %o %O %# %$ %%`), flat / mixed / object rows, `$variable` / `$variable.path` interpolation,
//! tagged templates, describe / skip / only / concurrent / failing `.each` and jest-each's table
//! errors. Each test throws `ARGS <its arguments>`, so the report records what it received.
//! The expected `.tsv` files are real Jest 30.5.2 (`jest --json`, `pending` written `skipped`)
//! and real vitest 5.1.0 (`vitest run --reporter=json`) on these same files:
//! `file \t status \t full name \t first line of the first failure message`.
//! `TURBO_TEST_COMPAT` / `--compat` override the per-file Jest-project detection.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const EACH: &str = include_str!("data/jest-each-085/each.test.js");
const JEST_ONLY: &str = include_str!("data/jest-each-085/jest-only.test.js");
const ONLY: &str = include_str!("data/jest-each-085/only.test.js");
const JEST: &str = include_str!("data/jest-each-085/jest-30.5.2.tsv");
const VITEST: &str = include_str!("data/jest-each-085/vitest-5.1.0.tsv");

fn jest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    p
}

fn vitest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "vitest.config.mjs",
        "export default { test: { globals: true, environment: 'node' } };",
    );
    p
}

/// The report as the oracle files list it (one line per test, files by name).
fn lines(p: &Project, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut all = vec!["--reporter", "json"];
    all.extend_from_slice(args);
    let run = turbo_test(&p.dir, &all, env);
    let report = json_report(&run);
    let mut files: Vec<&serde_json::Value> =
        report["testResults"].as_array().unwrap().iter().collect();
    files.sort_by_key(|f| f["name"].as_str().unwrap().to_string());
    files.iter().flat_map(|f| file_lines(f)).collect()
}

fn file_lines(f: &serde_json::Value) -> Vec<String> {
    let file = f["name"].as_str().unwrap().rsplit('/').next().unwrap();
    let tests = f["assertionResults"].as_array().unwrap();
    if tests.is_empty() {
        let msg = f["message"]
            .as_str()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");
        return vec![format!("{file}\tLOAD\t{msg}")];
    }
    tests.iter().map(|a| test_line(file, a)).collect()
}

fn test_line(file: &str, a: &serde_json::Value) -> String {
    let mut names: Vec<&str> = a["ancestorTitles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    names.push(a["title"].as_str().unwrap());
    let msg = a["failureMessages"][0].as_str().unwrap_or("");
    let first = msg.lines().next().unwrap_or("");
    format!(
        "{file}\t{}\t{}\t{first}",
        a["status"].as_str().unwrap(),
        names.join(" > ")
    )
}

/// The oracle's lines for `files` only.
fn expected(oracle: &str, files: &[&str]) -> Vec<String> {
    oracle
        .lines()
        .filter(|l| files.iter().any(|f| l.starts_with(&format!("{f}\t"))))
        .map(str::to_string)
        .collect()
}

/// Where turbo-test's vitest `.each` titles still read as vitest 4 (unchanged by the jest-each
/// work): vitest 5 prints a `$key` string value unquoted and an object `%s` / `$key` value
/// through its pretty-format based `inspect`. (vitest 5.1.0 line, turbo-test line.)
const VITEST5_TITLE_GAPS: [(&str, &str); 5] = [
    (
        "object 1 deep { c: 'deep' } $# undefined",
        "object 1 'deep' { c: 'deep' } $# undefined",
    ),
    (
        "object two [ 1 ] { c: [ 1 ] } $# undefined",
        "object 'two' [ 1 ] { c: [ 1 ] } $# undefined",
    ),
    ("tagged 1 x { k: { z: 1 } }", "tagged 1 'x' { k: { z: 1 } }"),
    (
        "{ a: null, b: [Array], obj: undefined }",
        "{ a: null, b: [ 1, 2 ], obj: undefined }",
    ),
    ("tsuite tx > inner", "tsuite 'tx' > inner"),
];

/// The vitest 5.1.0 oracle's lines for `files`, with VITEST5_TITLE_GAPS applied.
fn vitest_expected(files: &[&str]) -> Vec<String> {
    let gap = |l: String| {
        VITEST5_TITLE_GAPS
            .iter()
            .fold(l, |l, (vitest, tt)| l.replace(vitest, tt))
    };
    expected(VITEST, files).into_iter().map(gap).collect()
}

fn assert_lines(got: Vec<String>, want: Vec<String>) {
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "line {i}\n got: {got:#?}");
    }
    assert_eq!(got.len(), want.len(), "{got:#?}");
}

#[test]
fn jest_project_each_follows_jest_each() {
    let p = jest_project("jest-each-085");
    p.file("each.test.js", EACH);
    p.file("jest-only.test.js", JEST_ONLY);
    p.file("only.test.js", ONLY);
    assert_lines(
        lines(&p, &[], &[]),
        expected(JEST, &["each.test.js", "jest-only.test.js", "only.test.js"]),
    );
}

#[test]
fn vitest_project_each_keeps_vitest_titles() {
    let p = vitest_project("vitest-each-085");
    p.file("each.test.js", EACH);
    p.file("only.test.js", ONLY);
    assert_lines(
        lines(&p, &[], &[]),
        vitest_expected(&["each.test.js", "only.test.js"]),
    );
}

#[test]
fn compat_env_makes_a_vitest_project_follow_jest() {
    let p = vitest_project("compat-env-jest-085");
    p.file("each.test.js", EACH);
    p.file("only.test.js", ONLY);
    let got = lines(&p, &[], &[("TURBO_TEST_COMPAT", "jest")]);
    assert_lines(got, expected(JEST, &["each.test.js", "only.test.js"]));
}

#[test]
fn compat_flag_makes_a_jest_project_follow_vitest() {
    let p = jest_project("compat-flag-vitest-085");
    p.file("each.test.js", EACH);
    p.file("only.test.js", ONLY);
    let want = vitest_expected(&["each.test.js", "only.test.js"]);
    assert_lines(lines(&p, &["--compat", "vitest"], &[]), want.clone());
    // the one-token form, and the flag wins over the environment
    let got = lines(&p, &["--compat=vitest"], &[("TURBO_TEST_COMPAT", "jest")]);
    assert_lines(got, want);
}

#[test]
fn compat_auto_and_unknown_values_keep_detection() {
    let p = jest_project("compat-auto-085");
    p.file("each.test.js", EACH);
    let want = expected(JEST, &["each.test.js"]);
    assert_lines(lines(&p, &["--compat", "auto"], &[]), want.clone());
    // an unknown value is warned about and ignored
    let run = turbo_test(&p.dir, &["--compat", "mocha"], &[]);
    assert!(run.stderr.contains("--compat"), "{}", run.all());
    assert_lines(lines(&p, &[], &[("TURBO_TEST_COMPAT", "mocha")]), want);
}
