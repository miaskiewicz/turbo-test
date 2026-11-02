//! `--reporter json` against vitest 4.1.12's JSON reporter on the same project: the whole report
//! (key order included) with timings zeroed, failure messages cut to their first line (the stack
//! frames are runner-specific) and file names made project-relative.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project, Run};
use serde_json::Value;

/// The report's object keys in document order (string tokens followed by `:`).
fn keys(json: &str) -> Vec<String> {
    let b = json.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] == b'"' {
            let start = i + 1;
            i += 1;
            while b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b':' {
                out.push(json[start..i].to_string());
            }
        }
        i += 1;
    }
    out
}

/// The run's JSON report with timings zeroed, failure messages cut to their first line and file
/// names relative to the project, files in name order.
fn normalized(p: &Project, r: &Run) -> Value {
    let mut j: Value = serde_json::from_str(r.stdout.trim())
        .unwrap_or_else(|e| panic!("not a JSON report ({e}):\n{}", r.all()));
    j["startTime"] = 0.into();
    let prefix = format!("{}/", p.dir.display());
    let files = j["testResults"].as_array_mut().unwrap();
    for f in files.iter_mut() {
        f["startTime"] = 0.into();
        f["endTime"] = 0.into();
        let name = f["name"].as_str().unwrap().replace(&prefix, "");
        f["name"] = name.into();
        for a in f["assertionResults"].as_array_mut().unwrap() {
            if a.get("duration").is_some() {
                a["duration"] = 0.into();
            }
            for m in a["failureMessages"].as_array_mut().unwrap() {
                *m = m.as_str().unwrap().lines().next().unwrap_or("").into();
            }
        }
    }
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    j
}

/// A describe tree with passing / failing / skipped / todo / skipIf tests, a throwing beforeAll
/// and an unnamed describe; a file that fails to load; describe.skip / describe.todo blocks,
/// `.each` and an empty describe; and a `describe.only`.
const FILES: [(&str, &str); 4] = [
    (
        "a.test.js",
        r#"import { describe, it, test, expect, beforeAll } from 'vitest'
test('top pass', () => { expect(1).toBe(1) })
describe('outer', () => {
  it('inner pass', () => {})
  describe('deep', () => {
    it('deep fail', () => { throw new Error('boom') })
  })
  it.skip('skipped one', () => {})
  it.todo('todo one')
  it.skipIf(true)('skip if', () => {})
})
test('after describe', () => {})
describe('hooked', () => {
  beforeAll(() => { throw new TypeError('ba boom') })
  test('never runs', () => {})
})
describe('', () => { test('in unnamed', () => {}) })
"#,
    ),
    (
        "b.test.js",
        r#"import { test } from 'vitest'
throw new Error('load boom')
test('x', () => {})
"#,
    ),
    (
        "c.test.js",
        r#"import { test, describe } from 'vitest'
describe.skip('sk', () => { test('in skipped', () => {}); describe('sub', () => { test('in sub', () => {}) }) })
describe.todo('later')
describe.todo('later with body', () => { test('t', () => {}) })
test.each([1, 2])('each %i', () => {})
describe('empty', () => {})
"#,
    ),
    (
        "d.test.js",
        r#"import { test, describe } from 'vitest'
describe.only('focus', () => { test('runs', () => {}) })
test('not focused', () => {})
"#,
    ),
];

/// vitest 4.1.12 `--reporter json` on [`FILES`] (normalized as [`normalized`]): a plain run and a
/// `-t "outer|each"` run.
const VITEST: [&str; 2] = [
    r#"{"numTotalTestSuites":14,"numPassedTestSuites":5,"numFailedTestSuites":7,"numPendingTestSuites":2,"numTotalTests":16,"numPassedTests":7,"numFailedTests":1,"numPendingTests":6,"numTodoTests":2,"snapshot":{"added":0,"failure":false,"filesAdded":0,"filesRemoved":0,"filesRemovedList":[],"filesUnmatched":0,"filesUpdated":0,"matched":0,"total":0,"unchecked":0,"uncheckedKeysByFile":[],"unmatched":0,"updated":0,"didUpdate":false},"startTime":0,"success":false,"testResults":[{"assertionResults":[{"ancestorTitles":[],"fullName":"top pass","status":"passed","title":"top pass","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer inner pass","status":"passed","title":"inner pass","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer","deep"],"fullName":"outer deep deep fail","status":"failed","title":"deep fail","duration":0,"failureMessages":["Error: boom"],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer skipped one","status":"skipped","title":"skipped one","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer todo one","status":"todo","title":"todo one","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer skip if","status":"skipped","title":"skip if","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"after describe","status":"passed","title":"after describe","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["hooked"],"fullName":"hooked never runs","status":"skipped","title":"never runs","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[""],"fullName":" in unnamed","status":"passed","title":"in unnamed","duration":0,"failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"failed","message":"","name":"a.test.js"},{"assertionResults":[],"startTime":0,"endTime":0,"status":"failed","message":"load boom","name":"b.test.js"},{"assertionResults":[{"ancestorTitles":["sk"],"fullName":"sk in skipped","status":"skipped","title":"in skipped","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["sk","sub"],"fullName":"sk sub in sub","status":"skipped","title":"in sub","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["later with body"],"fullName":"later with body t","status":"todo","title":"t","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"each 1","status":"passed","title":"each 1","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"each 2","status":"passed","title":"each 2","duration":0,"failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"failed","message":"","name":"c.test.js"},{"assertionResults":[{"ancestorTitles":["focus"],"fullName":"focus runs","status":"passed","title":"runs","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"not focused","status":"skipped","title":"not focused","failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"passed","message":"","name":"d.test.js"}]}"#,
    r#"{"numTotalTestSuites":14,"numPassedTestSuites":6,"numFailedTestSuites":6,"numPendingTestSuites":2,"numTotalTests":16,"numPassedTests":3,"numFailedTests":1,"numPendingTests":10,"numTodoTests":2,"snapshot":{"added":0,"failure":false,"filesAdded":0,"filesRemoved":0,"filesRemovedList":[],"filesUnmatched":0,"filesUpdated":0,"matched":0,"total":0,"unchecked":0,"uncheckedKeysByFile":[],"unmatched":0,"updated":0,"didUpdate":false},"startTime":0,"success":false,"testResults":[{"assertionResults":[{"ancestorTitles":[],"fullName":"top pass","status":"skipped","title":"top pass","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer inner pass","status":"passed","title":"inner pass","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer","deep"],"fullName":"outer deep deep fail","status":"failed","title":"deep fail","duration":0,"failureMessages":["Error: boom"],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer skipped one","status":"skipped","title":"skipped one","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer todo one","status":"todo","title":"todo one","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["outer"],"fullName":"outer skip if","status":"skipped","title":"skip if","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"after describe","status":"skipped","title":"after describe","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["hooked"],"fullName":"hooked never runs","status":"skipped","title":"never runs","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[""],"fullName":" in unnamed","status":"skipped","title":"in unnamed","failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"failed","message":"","name":"a.test.js"},{"assertionResults":[],"startTime":0,"endTime":0,"status":"failed","message":"load boom","name":"b.test.js"},{"assertionResults":[{"ancestorTitles":["sk"],"fullName":"sk in skipped","status":"skipped","title":"in skipped","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["sk","sub"],"fullName":"sk sub in sub","status":"skipped","title":"in sub","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":["later with body"],"fullName":"later with body t","status":"todo","title":"t","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"each 1","status":"passed","title":"each 1","duration":0,"failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"each 2","status":"passed","title":"each 2","duration":0,"failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"failed","message":"","name":"c.test.js"},{"assertionResults":[{"ancestorTitles":["focus"],"fullName":"focus runs","status":"skipped","title":"runs","failureMessages":[],"meta":{},"tags":[]},{"ancestorTitles":[],"fullName":"not focused","status":"skipped","title":"not focused","failureMessages":[],"meta":{},"tags":[]}],"startTime":0,"endTime":0,"status":"passed","message":"","name":"d.test.js"}]}"#,
];

#[test]
fn json_report_matches_vitest() {
    // turbo-test#63 item 1: the JSON report had per-file counts only (no `assertionResults`, no
    // suite / pending / todo counts, no snapshot / startTime, `numPassingTests` per file and an
    // `error` status for a load error).
    for esbuild in [false, true] {
        let p = Project::new("json-vitest");
        if esbuild {
            p.with_esbuild();
        }
        for (name, src) in FILES {
            p.file(name, src);
        }
        for (args, want) in [(&[][..], VITEST[0]), (&["-t", "outer|each"][..], VITEST[1])] {
            let mut argv = vec!["--reporter", "json"];
            argv.extend_from_slice(args);
            let r = turbo_test(&p.dir, &argv, &[]);
            assert_eq!(r.code, Some(1), "{}", r.all());
            let want: Value = serde_json::from_str(want).unwrap();
            assert_eq!(
                normalized(&p, &r),
                want,
                "esbuild={esbuild} {args:?}\n{}",
                r.all()
            );
            assert_eq!(
                keys(r.stdout.trim()),
                keys(VITEST[usize::from(!args.is_empty())])
            );
        }
    }
}

/// Test locations (`--includeTaskLocation` / `test.includeTaskLocation`): plain calls, member
/// calls (`it.skip` — the property), call-of-call (`test.each(...)(` — its parenthesis, also
/// across lines), a non-ASCII prefix (UTF-16 columns), `.for`, and a TypeScript file whose
/// `vi.mock` is hoisted and whose mock factory shares a `let`.
const LOCATED: [(&str, &str); 3] = [
    (
        "a.test.js",
        r#"import { describe, it, test, expect } from 'vitest'
test('top', () => { expect(1).toBe(1) })
describe('d', () => {
  it('inner', () => {})
  it.skip('sk', () => {})
  it.todo('td')
    test.each([1, 2])('each %i', () => {})
  test.each([
    [1, 2],
  ])('multi %i', () => {})
})
const é = 'ü'; test('after unicode ' + é, () => {})
test.for([3])('for %i', () => {})
"#,
    ),
    (
        "dep.ts",
        r#"export const v = (): number => 1
"#,
    ),
    (
        "t.test.ts",
        r#"import { describe, it, test, expect, vi } from 'vitest'
import { v } from './dep'
interface Row { n: number }
const rows: Row[] = [{ n: 1 }]
let shared = 2
describe('typed', () => {
  test('uses mock', () => { expect(v()).toBe(2) })
  it.each(rows)('row $n', (r: Row) => { expect(r.n).toBe(1) })
})
test('last', (): void => {})
vi.mock('./dep', () => ({ v: () => shared }))
"#,
    ),
];

/// vitest 4.1.12's `location` for each test of [`LOCATED`], by file and full name.
const LOCATIONS: [(&str, &str, u64, u64); 12] = [
    ("a.test.js", "top", 2, 1),
    ("a.test.js", "d inner", 4, 3),
    ("a.test.js", "d sk", 5, 6),
    ("a.test.js", "d td", 6, 6),
    ("a.test.js", "d each 1", 7, 22),
    ("a.test.js", "d each 2", 7, 22),
    ("a.test.js", "d multi 1", 10, 5),
    ("a.test.js", "after unicode ü", 12, 16),
    ("a.test.js", "for 3", 13, 14),
    ("t.test.ts", "typed uses mock", 7, 3),
    ("t.test.ts", "typed row 1", 8, 16),
    ("t.test.ts", "last", 10, 1),
];

#[test]
fn json_report_has_vitest_task_locations() {
    // turbo-test#63 item 1: `--includeTaskLocation` was an unsupported flag and the report had no
    // `location`. vitest writes `location: { line, column }` (1-based, UTF-16 columns, the call's
    // V8 position mapped to the original source) after `failureMessages`.
    for esbuild in [false, true] {
        for via_config in [false, true] {
            let p = Project::new("json-location");
            if esbuild {
                p.with_esbuild();
            }
            for (name, src) in LOCATED {
                // without esbuild a mock factory cannot read the file's `let` (turbo-test#72);
                // same line layout either way
                let src = if esbuild {
                    src.to_string()
                } else {
                    src.replace("v: () => shared", "v: () => 2")
                };
                p.file(name, &src);
            }
            let mut args = vec!["--reporter", "json"];
            if via_config {
                p.file(
                    "vitest.config.js",
                    "export default { test: { includeTaskLocation: true } }\n",
                );
            } else {
                args.push("--includeTaskLocation");
            }
            let r = turbo_test(&p.dir, &args, &[]);
            assert_eq!(r.code, Some(0), "{}", r.all());
            let j = normalized(&p, &r);
            let mut got = Vec::new();
            for f in j["testResults"].as_array().unwrap() {
                for a in f["assertionResults"].as_array().unwrap() {
                    let loc = &a["location"];
                    got.push((
                        f["name"].as_str().unwrap().to_string(),
                        a["fullName"].as_str().unwrap().to_string(),
                        loc["line"].as_u64().unwrap_or(0),
                        loc["column"].as_u64().unwrap_or(0),
                    ));
                }
            }
            let want: Vec<_> = LOCATIONS
                .iter()
                .map(|(f, n, l, c)| (f.to_string(), n.to_string(), *l, *c))
                .collect();
            assert_eq!(
                got,
                want,
                "esbuild={esbuild} config={via_config}\n{}",
                r.all()
            );
            let k = keys(r.stdout.trim());
            let at = k.iter().position(|k| k == "location").unwrap();
            assert_eq!(
                &k[at - 1..at + 4],
                ["failureMessages", "location", "line", "column", "meta"]
            );
        }
    }
    // without the option there is no `location`
    let p = Project::new("json-no-location");
    p.file("a.test.js", LOCATED[0].1);
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert!(
        !keys(r.stdout.trim()).contains(&"location".to_string()),
        "{}",
        r.all()
    );
}
