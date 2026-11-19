//! turbo-test#403: the line diff of a failed assertion ran out of heap. `__diffSequences` (the
//! port of jest's diff-sequences) read the middle snake's common runs from the shared `division`
//! after recursing into the preceding half, which had overwritten them: a common run was reported
//! twice (or at the wrong indexes), and `__lineDiff`'s `ai !== ac` walk then never ended, pushing
//! until the isolate hit its heap limit. A real Flux spec whose `toThrow(new BadRequestException(..))`
//! failed on a body with a nested `params` aborted its whole file this way.
//!
//! The expected diffs are real vitest 5.1.0's `this.utils.diff(expected, received,
//! { omitAnnotationLines: true })` over the same line pairs (lines joined by `/` below), and its
//! reporter's diff of the failing `toEqual`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

/// `expected|received|diff`, each a list of lines joined by `/`.
const CORPUS: &str = "\
a/b|b/a/c/c|- a/  b/+ a/+ c/+ c
c/c/b/c|c/b/b/a/a|  c/- c/+ b/  b/- c/+ a/+ a
b|b/c/c|  b/+ c/+ c
b|a/c/c/c/b/a|+ a/+ c/+ c/+ c/  b/+ a
c/b/a/a/b|b/b/c|- c/  b/- a/- a/  b/+ c
a/c/c/c/b/c/a|b|- a/- c/- c/- c/  b/- c/- a
a/c/b/c|b/a/a/a/b/a|+ b/  a/- c/+ a/+ a/  b/- c/+ a
a/b/a/b/b|c|- a/- b/- a/- b/- b/+ c
b/c/c/a/a/c|c/c/a/c/b/b|- b/  c/  c/- a/  a/  c/+ b/+ b
a|c/b/a/c|+ c/+ b/  a/+ c
b/b/a/a/c|b/b/a/c/c/a|  b/  b/  a/+ c/+ c/  a/- c
b/c|a/c|- b/+ a/  c
c/b/b/b/b|b/b|- c/- b/- b/  b/  b
c/c/c/b/a|b/b/b/b|- c/- c/- c/  b/- a/+ b/+ b/+ b
a/c/a|b/b|- a/- c/- a/+ b/+ b
c/c/c/a|a|- c/- c/- c/  a
b|b/a/a|  b/+ a/+ a
c/c|c/c/b/a/a/a/a|  c/  c/+ b/+ a/+ a/+ a/+ a
a/b/b/a/c|a|  a/- b/- b/- a/- c
a/b/b|a/b|  a/  b/- b
a|a/a/a/c/b/a|  a/+ a/+ a/+ c/+ b/+ a
b/a/c/a/c|c/a/a/c/b/a/c|- b/+ c/+ a/  a/  c/+ b/  a/  c
b|b/a/b|  b/+ a/+ b
b|a/c/b/b|+ a/+ c/+ b/  b
b/b|a/a|- b/- b/+ a/+ a
b/c/c/b/a/c|a/b/a|- b/- c/- c/+ a/  b/  a/- c
a|a/b/b/c|  a/+ b/+ b/+ c
c/b/b/c/c/a/a|b/a/b/b/a|- c/+ b/+ a/  b/  b/- c/- c/- a/  a
a/b/c/b/c/b/a|b/b|- a/  b/- c/  b/- c/- b/- a
b/a|a/c/a/b/a/c|+ a/+ c/+ a/  b/  a/+ c
c/c/a/b/b/b|c/c/c/b/b|  c/  c/- a/- b/+ c/  b/  b
b/c/b/a/a/b|b/c/c/b/a|  b/  c/+ c/  b/  a/- a/- b
a/a/b/a/b|b/a/a/b|- a/- a/  b/+ a/  a/  b
a/c/c/c/b|a/a/b/b|  a/- c/- c/- c/+ a/+ b/  b
a/b/a/a/b|a/a/a|  a/- b/  a/  a/- b
c/b/a/c/a/a|b|- c/  b/- a/- c/- a/- a
b/a/c|a/b/c/a/c/a|+ a/  b/+ c/  a/  c/+ a
c/b/b/a|b/a/a/c/a/b|- c/  b/- b/+ a/+ a/+ c/  a/+ b
a/a/c/c/c/b/a|b/b/a/b/b|- a/+ b/+ b/  a/- c/- c/- c/+ b/  b/- a
c|c/c/b/b/a/b|  c/+ c/+ b/+ b/+ a/+ b
";

/// A heap limit far below the default, so that the old runaway walk fails the run in a moment
/// instead of growing to V8's default limit.
const HEAP: &[(&str, &str)] = &[("TURBO_MAX_HEAP_MB", "256")];

fn corpus_test() -> String {
    let rows: Vec<String> = CORPUS
        .lines()
        .map(|row| {
            let cols: Vec<String> = row
                .split('|')
                .map(|c| format!("{:?}", c.replace('/', "\n")))
                .collect();
            format!("[{}]", cols.join(", "))
        })
        .collect();
    format!(
        "const corpus = [\n{}\n];\n{}",
        rows.join(",\n"),
        r#"
test('line diffs match vitest for reordered and repeated lines', () => {
  let got;
  expect.extend({
    grab(received, expected) {
      got = this.utils.diff(expected, received, { omitAnnotationLines: true });
      return { pass: true, message: () => '' };
    },
  });
  const diffs = corpus.map(([expected, received]) => {
    expect(received).grab(expected);
    return got;
  });
  expect(diffs).toEqual(corpus.map((row) => row[2]));
});
"#
    )
}

#[test]
fn line_diffs_match_vitest_for_reordered_and_repeated_lines() {
    let p = Project::new("diff-sequences-403-corpus");
    p.file("package.json", "{}");
    p.file("diff.test.js", &corpus_test());
    let run = turbo_test(&p.dir, &["--reporter", "json"], HEAP);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

const FAILING: &str = r#"
test('a key added beside a shared one', () => {
  expect({ m: 1, p: { d: ['A'] } }).toEqual({ m: 1, e: 'x', s: 1 });
});
test('the file goes on after the failure', () => {
  expect(1).toBe(1);
});
"#;

#[test]
fn a_failed_assertion_with_reordered_keys_reports_its_diff() {
    let p = Project::new("diff-sequences-403-failure");
    p.file("package.json", "{}");
    p.file("keys.test.js", FAILING);
    let run = turbo_test(&p.dir, &["--reporter", "json"], HEAP);
    let report = json_report(&run);
    assert_eq!(report["numFailedTests"], 1, "{}", run.all());
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    // vitest's reporter prints the diff (its JSON failureMessages carry only the stack)
    let env = [HEAP[0], ("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")];
    let run = turbo_test(&p.dir, &[], &env);
    let diff = "- Expected\n+ Received\n\n  {\n-   \"e\": \"x\",\n    \"m\": 1,\n-   \"s\": 1,\n+   \"p\": {\n+     \"d\": [\n+       \"A\",\n+     ],\n+   },\n  }";
    assert!(run.all().contains(diff), "{}", run.all());
}
