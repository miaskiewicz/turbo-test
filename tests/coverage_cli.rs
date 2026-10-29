//! `--coverage` end to end: V8 precise coverage of real TS/JS sources, remapped through esbuild's
//! inline source maps, reported as lcov / json-summary / text / html, and gated by thresholds
//! (total and per-file), include/exclude globs, the ignore-file directive and the
//! "0 instrumented files is a hard FAIL" rule.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project, Run};

/// Every decision shape the branch pass knows, one function each. Line 1 holds a real function
/// (esbuild's module header maps to line 1); the interface (lines 8-10) is type-only and erased.
const BRANCHES_TS: &str = "\
export function ifElse(x: number): string {
  if (x > 0) {
    return 'pos';
  } else {
    return 'non-pos';
  }
}
interface Box {
  value: number;
}
export function sometimes(b: Box): number {
  let r = 0;
  if (b.value > 0) {
    r = 1;
  }
  return r;
}
export function always(x: number): number {
  if (x >= 0) return 1;
  return 2;
}
export function never(x: number): number {
  if (x < 0) {
    throw new Error('neg');
  }
  return x;
}
export function tern(x: number): string {
  return x ? 'y' : 'n';
}
export function both(a: unknown, b: unknown): unknown {
  return a && b;
}
export function nullish(a: unknown): unknown {
  return a ?? 'd';
}
export function loop(n: number): number {
  let s = 0;
  for (let i = 0; i < n; i++) {
    s += i;
  }
  while (s > 100) {
    s--;
  }
  return s;
}
export function neverCalled(): string {
  return 'x';
}
";

const SWITCH_TS: &str = "\
export function sw(k: string): number {
  switch (k) {
    case 'a':
      return 1;
    case 'b':
      return 2;
    default:
      return 0;
  }
}
";

const BRANCHES_TEST: &str = "\
import { ifElse, sometimes, always, never, tern, both, nullish, loop } from './src/branches';
import { sw } from './src/sw';
test('branches', () => {
  expect(ifElse(1)).toBe('pos');
  expect(ifElse(-1)).toBe('non-pos');
  expect(sometimes({ value: 1 })).toBe(1);
  expect(sometimes({ value: 0 })).toBe(0);
  expect(always(1)).toBe(1);
  expect(never(1)).toBe(1);
  expect(tern(1)).toBe('y');
  expect(both(1, 2)).toBe(2);
  expect(both(0, 2)).toBe(0);
  expect(nullish(null)).toBe('d');
  expect(loop(3)).toBe(3);
  expect(sw('a')).toBe(1);
});
";

/// A plain CommonJS source (no esbuild ESM interop prologue).
const CJS_JS: &str = "\
function twice(x) {
  return x * 2;
}
function never() {
  return 0;
}
module.exports = { twice, never };
";

fn esbuild_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p
}

fn read(p: &Project, rel: &str) -> String {
    std::fs::read_to_string(p.path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn json(p: &Project, rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(p, rel)).unwrap_or_else(|e| panic!("{rel} is not JSON: {e}"))
}

/// The lcov record (`SF:` .. `end_of_record`) for `file` (absolute path).
fn lcov_record(lcov: &str, file: &std::path::Path) -> String {
    let head = format!("SF:{}\n", file.display());
    let start = lcov
        .find(&head)
        .unwrap_or_else(|| panic!("no record for {}:\n{lcov}", file.display()));
    let end = start + lcov[start..].find("end_of_record\n").unwrap() + "end_of_record\n".len();
    lcov[start..end].to_string()
}

/// The `SF:` paths of every lcov record, in order.
fn lcov_files(lcov: &str) -> Vec<String> {
    lcov.lines()
        .filter_map(|l| l.strip_prefix("SF:"))
        .map(str::to_string)
        .collect()
}

fn lines_with<'a>(rec: &'a str, prefix: &str) -> Vec<&'a str> {
    rec.lines().filter(|l| l.starts_with(prefix)).collect()
}

fn assert_ok(r: &Run) {
    assert_eq!(r.code, Some(0), "{}", r.all());
}

fn assert_fail(r: &Run) {
    assert_eq!(r.code, Some(1), "{}", r.all());
}

#[test]
fn every_reporter_writes_its_artifact_with_exact_ts_metrics() {
    let p = esbuild_project("cov-reporters");
    p.file("src/branches.ts", BRANCHES_TS);
    p.file("src/sw.ts", SWITCH_TS);
    p.file("branches.test.ts", BRANCHES_TEST);
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage-reporter",
            "lcov,json-summary,text,html",
            "--coverage-dir",
            "out/cov",
        ],
        &[],
    );
    assert_ok(&r);
    let branches = p.path("src/branches.ts");
    let sw = p.path("src/sw.ts");

    // ---- lcov ----
    let lcov = read(&p, "out/cov/lcov.info");
    assert_eq!(
        lcov_files(&lcov),
        vec![branches.display().to_string(), sw.display().to_string()]
    );
    let rec = lcov_record(&lcov, &branches);
    assert_eq!(
        lines_with(&rec, "BRDA:"),
        vec![
            "BRDA:2,0,0,1",
            "BRDA:2,0,1,1", // if/else, both arms
            "BRDA:13,1,0,1",
            "BRDA:13,1,1,1", // if without else, then taken once
            "BRDA:19,2,0,1",
            "BRDA:19,2,1,0", // braceless `if … return`, always taken
            "BRDA:23,3,0,0",
            "BRDA:23,3,1,1", // if that never throws
            "BRDA:29,4,0,1",
            "BRDA:29,4,1,0", // ternary, only the consequent
            "BRDA:32,5,0,2",
            "BRDA:32,5,1,1", // `a && b`: right side once
            "BRDA:35,6,0,1",
            "BRDA:35,6,1,1", // `a ?? 'd'` on null
        ]
    );
    assert!(rec.contains("BRF:14\nBRH:11\n"), "{rec}");
    // lines: the erased interface (8-10) is not executable; uncovered = never-run code.
    let da: Vec<&str> = lines_with(&rec, "DA:");
    assert_eq!(da.len(), 40, "{rec}");
    let uncovered: Vec<&str> = da.iter().filter(|l| l.ends_with(",0")).copied().collect();
    assert_eq!(
        uncovered,
        vec!["DA:20,0", "DA:24,0", "DA:43,0", "DA:47,0", "DA:48,0", "DA:49,0"]
    );
    assert!(
        !da.iter()
            .any(|l| l.starts_with("DA:8,") || l.starts_with("DA:9,")),
        "{rec}"
    );
    assert!(da.contains(&"DA:1,2") && da.contains(&"DA:40,3"), "{rec}"); // call / loop counts
    assert!(rec.contains("LF:40\nLH:34\nend_of_record\n"), "{rec}");
    // functions: the real declarations with their call counts
    for f in [
        "FN:11,sometimes",
        "FN:22,never",
        "FN:37,loop",
        "FN:47,neverCalled",
    ] {
        assert!(rec.lines().any(|l| l == f), "{f} missing:\n{rec}");
    }
    assert!(rec.lines().any(|l| l == "FNDA:0,neverCalled"), "{rec}");
    assert!(rec.lines().any(|l| l == "FNDA:2,sometimes"), "{rec}");
    let sw_rec = lcov_record(&lcov, &sw);
    assert_eq!(
        lines_with(&sw_rec, "BRDA:"),
        vec!["BRDA:2,0,0,1", "BRDA:2,0,1,0", "BRDA:2,0,2,0"]
    );
    assert!(sw_rec.contains("BRF:3\nBRH:1\n"), "{sw_rec}");

    // ---- json-summary ----
    let j = json(&p, "out/cov/coverage-summary.json");
    let f = &j[branches.to_str().unwrap()];
    let m = |metric: &str| {
        (
            f[metric]["total"].as_u64().unwrap(),
            f[metric]["covered"].as_u64().unwrap(),
        )
    };
    assert_eq!(m("lines"), (40, 34));
    assert_eq!(m("statements"), (23, 19));
    assert_eq!(m("branches"), (14, 11));
    assert_eq!(f["lines"]["pct"], 85.0);
    assert_eq!(f["statements"]["pct"], 82.61);
    assert_eq!(f["branches"]["skipped"], 0);
    assert_eq!(j["total"]["branches"]["total"], 17);
    assert_eq!(j["total"]["branches"]["covered"], 12);
    assert_eq!(j["total"]["statements"]["total"], 27);
    assert_eq!(j["total"]["statements"]["covered"], 21);
    assert_eq!(j.as_object().unwrap().len(), 3, "total + 2 files: {j}");

    // ---- html ----
    let html = read(&p, "out/cov/index.html");
    assert!(html.contains("<h1>Coverage — 2 files</h1>"), "{html}");
    assert!(
        html.contains(
            "<td class=\"file\">src/branches.ts</td>\
<td class=\"med\">85.00% <span class=\"frac\">(34/40)</span></td>\
<td class=\"med\">82.61% <span class=\"frac\">(19/23)</span></td>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<td class=\"med\">78.57% <span class=\"frac\">(11/14)</span></td></tr>"),
        "{html}"
    );
    assert!(html.contains("<td class=\"file\">src/sw.ts</td>"), "{html}");

    // ---- text ----
    assert!(
        r.stdout
            .contains("\n Coverage — 2 files (lines | funcs | branches | stmts)\n"),
        "{}",
        r.all()
    );
    let row = r
        .stdout
        .lines()
        .find(|l| l.ends_with("   src/branches.ts"))
        .expect("branches row");
    assert!(row.starts_with("   85.00% ln"), "{row}");
    assert!(
        row.ends_with("  78.57% br   82.61% st   src/branches.ts"),
        "{row}"
    );
    assert!(r.stdout.contains("12/17)"), "{}", r.stdout); // total branches
    assert!(
        r.stdout
            .contains("  → out/cov/lcov.info  out/cov/coverage-summary.json  out/cov/index.html\n"),
        "{}",
        r.stdout
    );
}

#[test]
fn default_reporters_are_lcov_json_and_text_under_coverage_dir() {
    let p = esbuild_project("cov-defaults");
    p.file("src/c.js", CJS_JS);
    p.file("c.test.ts", "const { twice } = require('./src/c.js');\ntest('c', () => { expect(twice(2)).toBe(4); });\n");
    let r = turbo_test(&p.dir, &["--coverage"], &[]);
    assert_ok(&r);
    let file = p.path("src/c.js");
    assert_eq!(
        read(&p, "coverage/lcov.info"),
        format!(
            "SF:{}\nFN:1,twice\nFN:4,never\nFNDA:1,twice\nFNDA:0,never\nFNF:2\nFNH:1\n\
DA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nDA:5,0\nDA:6,0\nDA:7,1\nLF:7\nLH:4\nend_of_record\n",
            file.display()
        )
    );
    let j = json(&p, "coverage/coverage-summary.json");
    assert_eq!(j["total"]["functions"]["pct"], 50.0);
    assert_eq!(j["total"]["lines"]["pct"], 57.14);
    assert!(!p.path("coverage/index.html").exists(), "html is opt-in");
    let cov = p.path("coverage");
    assert!(
        r.stdout
            .contains("   57.14% lines (4/7)    50.00% fns (1/2)   100.00% branches (0/0)"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains(&format!(
            "  → {}  {}\n",
            cov.join("lcov.info").display(),
            cov.join("coverage-summary.json").display()
        )),
        "{}",
        r.stdout
    );
    // no thresholds configured → no gate output at all
    assert!(!r.stdout.contains("coverage threshold"), "{}", r.stdout);
}

#[test]
fn reporter_selection_json_alias_text_only_and_blank_list() {
    let p = esbuild_project("cov-reporter-sel");
    p.file("src/c.js", CJS_JS);
    p.file("c.test.ts", "const { twice } = require('./src/c.js');\ntest('c', () => { expect(twice(1)).toBe(2); });\n");

    // `json` is an alias of json-summary; without `text` nothing is printed
    let r = turbo_test(
        &p.dir,
        &["--coverage-reporter", "json", "--coverage-dir", "a"],
        &[],
    );
    assert_ok(&r);
    assert!(p.path("a/coverage-summary.json").exists());
    assert!(!p.path("a/lcov.info").exists());
    assert!(!r.stdout.contains("Coverage —"), "{}", r.stdout);

    // text only (case-insensitive, whitespace-tolerant): a summary but no files and no `→` line
    let r = turbo_test(
        &p.dir,
        &["--coverage-reporters", " TEXT ", "--coverage-dir", "b"],
        &[],
    );
    assert_ok(&r);
    assert!(r.stdout.contains("\n Coverage — 1 files"), "{}", r.stdout);
    assert!(!r.stdout.contains('→'), "{}", r.stdout);
    assert_eq!(
        std::fs::read_dir(p.path("b")).unwrap().count(),
        0,
        "text writes no files"
    );

    // a list with no names keeps the default set
    let r = turbo_test(
        &p.dir,
        &["--coverage-reporter", " , ", "--coverage-dir", "c"],
        &[],
    );
    assert_ok(&r);
    assert!(p.path("c/lcov.info").exists() && p.path("c/coverage-summary.json").exists());
    assert!(r.stdout.contains("Coverage — 1 files"), "{}", r.stdout);
}

#[test]
fn coverage_dir_absolute_and_nested_is_created() {
    let p = esbuild_project("cov-absdir");
    p.file("src/c.js", CJS_JS);
    p.file("c.test.ts", "const { twice } = require('./src/c.js');\ntest('c', () => { expect(twice(1)).toBe(2); });\n");
    let out = p.path("deep/er/report");
    let r = turbo_test(&p.dir, &["--coverage-dir", out.to_str().unwrap()], &[]);
    assert_ok(&r);
    assert!(out.join("lcov.info").exists() && out.join("coverage-summary.json").exists());
    assert!(
        !p.path("coverage").exists(),
        "default dir unused when --coverage-dir is given"
    );
}

/// Two sources: `full.js` entirely covered, `part.js` half covered (lines 4/8, functions 1/2,
/// branches 1/2).
fn threshold_project(name: &str) -> Project {
    let p = esbuild_project(name);
    p.file(
        "src/full.js",
        "function full(x) {\n  return x + 1;\n}\nmodule.exports = { full };\n",
    );
    p.file(
        "src/part.js",
        "function used(x) {\n  return x ? 1 : 2;\n}\nfunction unused() {\n  const a = 1;\n  return a;\n}\nmodule.exports = { used, unused };\n",
    );
    p.file(
        "t.test.ts",
        "const { full } = require('./src/full.js');\nconst { used } = require('./src/part.js');\n\
test('t', () => { expect(full(1)).toBe(2); expect(used(1)).toBe(1); });\n",
    );
    p
}

#[test]
fn thresholds_pass_fail_and_ignore_malformed_parts() {
    let p = threshold_project("cov-thresholds");
    let thr = |spec: &str| {
        turbo_test(
            &p.dir,
            &["--coverage-reporter", "lcov", "--coverage-thresholds", spec],
            &[],
        )
    };

    // sanity: the part.js metrics the specs below are built around
    let r = thr("lines=0");
    assert_ok(&r);
    assert!(
        r.stdout.contains("  coverage thresholds met ✓\n"),
        "{}",
        r.stdout
    );
    let lcov = read(&p, "coverage/lcov.info");
    let part = lcov_record(&lcov, &p.path("src/part.js"));
    assert!(
        part.contains("LF:8\nLH:4\n")
            && part.contains("FNF:2\nFNH:1\n")
            && part.contains("BRF:2\nBRH:1\n"),
        "{part}"
    );
    assert!(
        lcov_record(&lcov, &p.path("src/full.js")).contains("LF:4\nLH:4\n"),
        "{lcov}"
    );

    // total lines = 8/12 = 66.67%: a 90% gate fails the run even though every test passed
    let r = thr("lines=90,branches=50");
    assert_fail(&r);
    assert!(r.stdout.contains("1 passed"), "{}", r.all());
    assert!(
        r.stdout
            .contains("\n  coverage threshold FAILURES (1):\n    ✗ total: lines 66.67% < 90%\n"),
        "{}",
        r.stdout
    );

    // malformed parts are skipped: only `funcs=100` gates (functions 2/3 = 66.67%)
    let r = thr("lines=abc,nonsense,foo=1,=5,funcs=100");
    assert_fail(&r);
    assert!(
        r.stdout
            .contains("coverage threshold FAILURES (1):\n    ✗ total: functions 66.67% < 100%\n"),
        "{}",
        r.stdout
    );

    // `--coverage-threshold` (singular) + repeated specs merge: lines=1 passes, stmts fails
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage-reporter",
            "text",
            "--coverage-threshold",
            "lines=1",
            "--coverage-thresholds",
            "stmts=100,branches=100",
        ],
        &[],
    );
    assert_fail(&r);
    assert!(
        r.stdout.contains("coverage threshold FAILURES (2):"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("    ✗ total: branches 50.00% < 100%\n"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("    ✗ total: statements "),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("✗ total: lines"), "{}", r.stdout);
}

#[test]
fn per_file_thresholds_name_only_the_failing_file() {
    let p = threshold_project("cov-perfile");
    // total lines 66.67% passes a 60% gate; part.js alone (50%) does not, full.js (100%) does.
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage-reporter",
            "lcov",
            "--coverage-per-file",
            "--coverage-thresholds",
            "lines=60,functions=60",
        ],
        &[],
    );
    assert_fail(&r);
    assert!(
        r.stdout.contains(
            "\n  coverage threshold FAILURES (2):\n    ✗ src/part.js: lines 50.00% < 60%\n    ✗ src/part.js: functions 50.00% < 60%\n"
        ),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("full.js"), "{}", r.stdout);
    // without --coverage-per-file the same gate passes on the total
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage-reporter",
            "lcov",
            "--coverage-thresholds",
            "lines=60,functions=60",
        ],
        &[],
    );
    assert_ok(&r);
    assert!(
        r.stdout.contains("coverage thresholds met ✓"),
        "{}",
        r.stdout
    );
}

fn globs_project(name: &str) -> Project {
    let p = esbuild_project(name);
    p.file(
        "src/a.ts",
        "export function a(): number {\n  return 1;\n}\n",
    );
    p.file(
        "src/deep/b.tsx",
        "export function b(): number {\n  return 2;\n}\n",
    );
    p.file(
        "lib/c.js",
        "function c() {\n  return 3;\n}\nmodule.exports = { c };\n",
    );
    p.file("lib/ignored.js", "// turbo-test-coverage-ignore-file\nfunction d() {\n  return 4;\n}\nmodule.exports = { d };\n");
    p.file(
        "g.test.ts",
        "import { a } from './src/a';\nimport { b } from './src/deep/b';\nconst { c } = require('./lib/c.js');\n\
const { d } = require('./lib/ignored.js');\ntest('g', () => { expect(a() + b() + c() + d()).toBe(10); });\n",
    );
    p
}

#[test]
fn include_exclude_globs_and_ignore_directive_filter_reported_files() {
    let p = globs_project("cov-globs");
    let files = |args: &[&str]| -> Vec<String> {
        let mut a = vec!["--coverage-reporter", "lcov"];
        a.extend_from_slice(args);
        let r = turbo_test(&p.dir, &a, &[]);
        assert_ok(&r);
        lcov_files(&read(&p, "coverage/lcov.info"))
            .into_iter()
            .map(|f| {
                f.strip_prefix(&format!("{}/", p.dir.display()))
                    .unwrap()
                    .to_string()
            })
            .collect()
    };
    // no globs: everything except the file carrying the ignore directive
    assert_eq!(files(&[]), vec!["lib/c.js", "src/a.ts", "src/deep/b.tsx"]);
    // the canonical vitest brace glob is one glob, not two
    assert_eq!(
        files(&["--coverage-include", "src/**/*.{ts,tsx}"]),
        vec!["src/a.ts", "src/deep/b.tsx"]
    );
    // top-level commas separate globs; excludes apply after includes
    assert_eq!(
        files(&[
            "--coverage-include",
            "src/*.ts,lib/**",
            "--coverage-exclude",
            "lib/c.*"
        ]),
        vec!["src/a.ts"]
    );
    // repeated flags accumulate
    assert_eq!(
        files(&[
            "--coverage-exclude",
            "**/*.tsx",
            "--coverage-exclude",
            "lib/?.js"
        ]),
        vec!["src/a.ts"]
    );
}

#[test]
fn zero_instrumented_files_is_a_hard_failure() {
    // include globs that match nothing
    let p = globs_project("cov-zero-include");
    let r = turbo_test(
        &p.dir,
        &["--coverage-include", "nope/**,other/*.{ts,js}"],
        &[],
    );
    assert_fail(&r);
    assert!(r.stdout.contains("1 passed"), "{}", r.all());
    assert!(
        r.stderr
            .contains("\n  coverage ERROR: 0 source files instrumented.\n"),
        "{}",
        r.all()
    );
    assert!(
        r.stderr.contains(
            "    The coverage.include globs matched no loaded source: [nope/**, other/*.{ts,js}]\n"
        ),
        "{}",
        r.all()
    );
    assert!(
        r.stderr.contains("Refusing to report a vacuous 0/0 pass"),
        "{}",
        r.all()
    );
    assert!(!p.path("coverage/lcov.info").exists());

    // a suite that loads no source at all
    let q = esbuild_project("cov-zero-none");
    q.file("only.test.ts", "test('x', () => { expect(1).toBe(1); });\n");
    let r = turbo_test(&q.dir, &["--coverage"], &[]);
    assert_fail(&r);
    assert!(
        r.stderr
            .contains("    No instrumentable source was loaded under --coverage.\n"),
        "{}",
        r.all()
    );

    // without esbuild no inline source map is emitted, so nothing can be attributed
    let n = Project::new("cov-zero-noesbuild");
    n.file(
        "src/a.ts",
        "export function a(): number {\n  return 1;\n}\n",
    );
    n.file(
        "a.test.ts",
        "import { a } from './src/a';\ntest('a', () => { expect(a()).toBe(1); });\n",
    );
    let r = turbo_test(&n.dir, &["--coverage"], &[]);
    assert_fail(&r);
    assert!(r.stdout.contains("1 passed"), "{}", r.all());
    assert!(
        r.stderr.contains("No instrumentable source was loaded"),
        "{}",
        r.all()
    );
}

#[test]
fn coverage_merges_across_test_files_by_max_count() {
    let p = esbuild_project("cov-merge");
    p.file(
        "src/m.ts",
        "export function ifElse(x: number): string {\n  if (x > 0) {\n    return 'pos';\n  } else {\n    return 'non-pos';\n  }\n}\n",
    );
    p.file(
        "pos.test.ts",
        "import { ifElse } from './src/m';\ntest('p', () => { expect(ifElse(1)).toBe('pos'); });\n",
    );
    p.file("neg.test.ts", "import { ifElse } from './src/m';\ntest('n', () => { expect(ifElse(-1)).toBe('non-pos'); });\n");
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage-reporter",
            "lcov,text",
            "--coverage-thresholds",
            "lines=100,branches=100,functions=100,statements=100",
        ],
        &[],
    );
    assert_ok(&r);
    // each file alone covers one arm; together both — and counts merge by max, not sum
    assert_eq!(
        read(&p, "coverage/lcov.info"),
        format!(
            "SF:{}\nFN:1,ifElse\nFNDA:1,ifElse\nFNF:1\nFNH:1\nBRDA:2,0,0,1\nBRDA:2,0,1,1\nBRF:2\nBRH:2\n\
DA:1,1\nDA:2,1\nDA:3,1\nDA:5,1\nDA:7,1\nLF:5\nLH:5\nend_of_record\n",
            p.path("src/m.ts").display()
        )
    );
    assert!(
        r.stdout
            .contains("  100.00% ln  100.00% fn  100.00% br  100.00% st   src/m.ts\n"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("coverage thresholds met ✓"),
        "{}",
        r.stdout
    );
}

#[test]
fn failing_tests_still_report_coverage_and_exit_nonzero() {
    let p = esbuild_project("cov-failing");
    p.file("src/c.js", CJS_JS);
    p.file("c.test.ts", "const { twice } = require('./src/c.js');\ntest('c', () => { expect(twice(2)).toBe(5); });\n");
    let r = turbo_test(&p.dir, &["--coverage", "--coverage-reporter", "lcov"], &[]);
    assert_fail(&r);
    assert!(read(&p, "coverage/lcov.info").contains("FNDA:1,twice\n"));
}

// ---- turbo-test#34: instrumentation accuracy ---------------------------------------------------
//
// The issue's repro project. Every expected value below is what vitest 4.1's v8 provider
// (@vitest/coverage-v8) reports for the same sources, except where a comment says otherwise.

/// Two ESM exports, one never called; esbuild turns each export into a named getter arrow.
const I34_LIB: &str =
    "export function used() { return 1 }\n\n\nexport function unused() { return 2 }\n";
/// Case bodies on the `case` line; only `case 'a'` runs.
const I34_SW: &str = "\
export function sw(x) {
  switch (x) {
    case 'a': return 1;
    case 'b': return 2;
    default: return 0;
  }
}
";
/// A braceless `if … throw` that never fires.
const I34_THR: &str = "export function g(c) {\n  if (c) throw new Error('x');\n  return 1;\n}\n";
/// Top-level CommonJS statements only.
const I34_TOP: &str = "const a = 1;\nconst b = a + 1;\nmodule.exports = { b };\n";
const I34_TEST: &str = "\
import { used } from './lib.js';
import { sw } from './sw.js';
import { g } from './thr.js';
const { b } = require('./top.cjs');
test('t', () => { expect(used()).toBe(1); expect(sw('a')).toBe(1); expect(g(false)).toBe(1); expect(b).toBe(2); });
";

/// Run the #34 repro (`--coverage --coverage-reporter lcov,json-summary a.test.js`); returns the
/// project, its lcov text and its coverage-summary.json.
fn issue34_run(name: &str) -> (Project, String, serde_json::Value) {
    let p = esbuild_project(name);
    p.file("package.json", "{\"name\":\"k1\"}");
    p.file("lib.js", I34_LIB);
    p.file("sw.js", I34_SW);
    p.file("thr.js", I34_THR);
    p.file("top.cjs", I34_TOP);
    p.file("a.test.js", I34_TEST);
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage",
            "--coverage-reporter",
            "lcov,json-summary",
            "a.test.js",
        ],
        &[],
    );
    assert_ok(&r);
    let lcov = read(&p, "coverage/lcov.info");
    let j = json(&p, "coverage/coverage-summary.json");
    (p, lcov, j)
}

/// `(covered, total)` of `metric` for `file` in a coverage-summary.json.
fn summary_of(j: &serde_json::Value, file: &std::path::Path, metric: &str) -> (u64, u64) {
    let m = &j[file.to_str().unwrap()][metric];
    (m["covered"].as_u64().unwrap(), m["total"].as_u64().unwrap())
}

#[test]
fn esm_export_getters_are_not_counted_as_functions() {
    let (p, lcov, j) = issue34_run("cov-i34-getters");
    let rec = lcov_record(&lcov, &p.path("lib.js"));
    // esbuild's `__export` getters (`unused: () => unused`) map to line 1; they are not functions
    // of the source, so `unused` is listed once, at its declaration.
    assert_eq!(
        rec.lines()
            .filter(|l| l.starts_with("FN"))
            .collect::<Vec<_>>(),
        vec![
            "FN:1,used",
            "FN:4,unused",
            "FNDA:1,used",
            "FNDA:0,unused",
            "FNF:2",
            "FNH:1"
        ]
    );
    assert_eq!(summary_of(&j, &p.path("lib.js"), "functions"), (1, 2));
}

#[test]
fn esm_prologue_does_not_make_a_type_only_or_blank_line_1_executable() {
    let p = esbuild_project("cov-i34-line1");
    p.file(
        "src/typed.ts",
        "type N = number;\nexport function f(x: N): N {\n  return x;\n}\n",
    );
    p.file(
        "src/blank.ts",
        "\nexport function h(): number {\n  return 3;\n}\n",
    );
    p.file(
        "t.test.ts",
        "import { f } from './src/typed';\nimport { h } from './src/blank';\n\
test('t', () => { expect(f(1)).toBe(1); expect(h()).toBe(3); });\n",
    );
    let r = turbo_test(&p.dir, &["--coverage", "--coverage-reporter", "lcov"], &[]);
    assert_ok(&r);
    let lcov = read(&p, "coverage/lcov.info");
    for file in ["src/typed.ts", "src/blank.ts"] {
        let rec = lcov_record(&lcov, &p.path(file));
        // vitest reports DA:3 only; turbo-test also counts the declaration and closing-brace
        // lines (its c8-style line model) — but never the type-only / blank line 1.
        assert_eq!(
            lines_with(&rec, "DA:"),
            vec!["DA:2,1", "DA:3,1", "DA:4,1"],
            "{file}"
        );
        assert!(rec.contains("LF:3\nLH:3\n"), "{rec}");
    }
}

#[test]
fn top_level_statements_are_covered() {
    let (p, lcov, j) = issue34_run("cov-i34-top");
    let top = p.path("top.cjs");
    // V8 counts top-level code only in the module-wide range, which opens on the wrapper line
    assert_eq!(summary_of(&j, &top, "statements"), (3, 3));
    assert_eq!(summary_of(&j, &top, "lines"), (3, 3));
    assert_eq!(
        lines_with(&lcov_record(&lcov, &top), "DA:"),
        vec!["DA:1,1", "DA:2,1", "DA:3,1"]
    );
}

#[test]
fn braceless_if_throw_that_never_fires_is_an_uncovered_statement() {
    let (p, lcov, j) = issue34_run("cov-i34-throw");
    let thr = p.path("thr.js");
    // the `throw` never ran: 2/3, the same as the braced `if (c) { throw … }` gives
    assert_eq!(summary_of(&j, &thr, "statements"), (2, 3));
    assert_eq!(
        lines_with(&lcov_record(&lcov, &thr), "BRDA:"),
        vec!["BRDA:2,0,0,0", "BRDA:2,0,1,1"]
    );
}

#[test]
fn lines_inside_never_run_switch_cases_are_uncovered() {
    let (p, lcov, j) = issue34_run("cov-i34-switch");
    let rec = lcov_record(&lcov, &p.path("sw.js"));
    // only `case 'a'` ran. vitest reports DA:2-5; turbo-test also counts the declaration and the
    // function's closing line (its c8-style line model).
    assert_eq!(
        lines_with(&rec, "DA:"),
        vec!["DA:1,1", "DA:2,1", "DA:3,1", "DA:4,0", "DA:5,0", "DA:7,1"]
    );
    assert_eq!(
        lines_with(&rec, "BRDA:"),
        vec!["BRDA:2,0,0,1", "BRDA:2,0,1,0", "BRDA:2,0,2,0"]
    );
    assert_eq!(summary_of(&j, &p.path("sw.js"), "lines"), (4, 6));
    assert_eq!(summary_of(&j, &p.path("sw.js"), "statements"), (2, 4));
}

/// turbo-test#40's repro: a braced `if` with no `else` whose consequent always runs.
const I40_K: &str = "export function k(c) {\n  if (c) {\n    c++;\n  }\n  return c;\n}\n";
/// More `if`-without-`else` shapes: an always-taken top-level `if`, one followed by a nested
/// function, one in a loop, a braceless nested pair, an else-if chain, and a never-taken one.
const I40_SHAPES: &str = "\
const on = !globalThis.__off;
if (on) {
  globalThis.__seen = 1;
}
export function later(c) {
  if (c) {
    c++;
  }
  const twice = () => c * 2;
  return twice();
}
export function loop(c) {
  let n = 0;
  for (let i = 0; i < 3; i++) {
    if (c) n++;
  }
  return n;
}
export function nested(c, d) {
  if (c)
    if (d) c++;
  return c;
}
export function chain(c) {
  if (c === 1) {
    c = 'a';
  } else if (c === 2) {
    c = 'b';
  }
  return c;
}
export function never(c) {
  if (c) {
    c++;
  }
  return c;
}
";
const I40_TEST: &str = "\
import { k } from './k.js';
import { later, loop, nested, chain, never } from './shapes.js';
test('k', () => { expect(k(1)).toBe(2); });
test('shapes', () => {
  expect(later(1)).toBe(4);
  expect(loop(1)).toBe(3);
  expect([nested(1, 1), nested(1, 0), nested(0, 1)]).toEqual([2, 1, 0]);
  expect(chain(2)).toBe('b');
  expect(never(0)).toBe(0);
});
";

#[test]
fn always_taken_if_without_else_counts_the_then_arm() {
    let p = esbuild_project("cov-i40-always");
    p.file("k.js", I40_K);
    p.file("shapes.js", I40_SHAPES);
    p.file("a.test.js", I40_TEST);
    let r = turbo_test(
        &p.dir,
        &["--coverage", "--coverage-reporter", "lcov", "a.test.js"],
        &[],
    );
    assert_ok(&r);
    let lcov = read(&p, "coverage/lcov.info");
    // V8 drops a consequent range whose count equals the enclosing one (it always ran): then=1
    assert_eq!(
        lines_with(&lcov_record(&lcov, &p.path("k.js")), "BRDA:"),
        vec!["BRDA:2,0,0,1", "BRDA:2,0,1,0"]
    );
    // every value below is what vitest 4.1.12's v8 provider reports for the same files
    assert_eq!(
        lines_with(&lcov_record(&lcov, &p.path("shapes.js")), "BRDA:"),
        vec![
            "BRDA:2,0,0,1",
            "BRDA:2,0,1,0", // top-level, always taken
            "BRDA:6,1,0,1",
            "BRDA:6,1,1,0", // always taken, a called arrow function after it
            "BRDA:15,2,0,3",
            "BRDA:15,2,1,0", // braceless, in a loop, taken on every iteration
            "BRDA:20,3,0,2",
            "BRDA:20,3,1,1", // outer braceless `if (c)`: twice of three
            "BRDA:21,4,0,1",
            "BRDA:21,4,1,1", // inner `if (d)`: once of two
            "BRDA:25,5,0,0",
            "BRDA:25,5,1,1", // `c === 1` false
            "BRDA:27,6,0,1",
            "BRDA:27,6,1,0", // `else if (c === 2)`: always taken when reached
            "BRDA:33,7,0,0",
            "BRDA:33,7,1,1", // never taken: V8 keeps its 0-count consequent range
        ]
    );
}
