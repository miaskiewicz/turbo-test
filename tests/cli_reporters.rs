//! Every reporter (default/dot/verbose/json/junit/tap + an unknown one) and `--outputFile`
//! against one project holding a partially failing file (with a skipped test and XML-special
//! names), a passing file, and a file that fails to load.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project, Run};

fn fixture(name: &str) -> Project {
    let p = Project::new(name);
    p.file(
        "a.test.ts",
        "test('adds <&>', () => { expect(1 + 1).toBe(2) })\n\
         test.skip('later', () => {})\n\
         test('breaks', () => { expect('x').toBe('y') })\n",
    );
    p.file(
        "b.test.ts",
        "test('ok', () => { expect(true).toBe(true) })\n",
    );
    p.file("c.test.ts", "throw new Error('top <boom>')\n");
    p
}

fn path(p: &Project, rel: &str) -> String {
    p.path(rel).display().to_string()
}

fn run(p: &Project, args: &[&str]) -> Run {
    let r = turbo_test(&p.dir, args, &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    r
}

const SUMMARY: &str = "3 files | 2 passed | 1 failed | 1 load-errors | 2 jobs | wall ";
const FOOTER: &str =
    "\n Test Files  2 failed | 1 passed (3)\n      Tests  1 failed | 2 passed | 1 skipped (4)\n   Duration  ";

#[test]
fn default_text_reporter_and_unknown_fallback() {
    let p = fixture("rep-default");
    let lines = format!(
        "FAIL  {}  (1 passed, 1 failed)\n        ✗ breaks: expected 'x' to be 'y' // Object.is equality\nPASS  {}  (1 passed, 0 failed)\nERROR {}  (module errored: Error: top <boom>)\n",
        String::from("a.test.ts"),
        String::from("b.test.ts"),
        String::from("c.test.ts")
    );
    for args in [
        &[][..],
        &["--reporter", "default"],
        &["--reporter", "html"],
        &["--reporters", "hanging-process"],
    ] {
        let r = run(&p, args);
        assert!(r.stdout.starts_with(&lines), "{args:?}\n{}", r.all());
        assert!(r.stdout.contains(FOOTER), "{}", r.all());
        assert!(r.stdout.contains(SUMMARY), "{}", r.all());
        assert!(!r.stderr.contains(SUMMARY), "{}", r.all());
    }
}

#[test]
fn dot_and_verbose_reporters() {
    let p = fixture("rep-dot");
    let r = run(&p, &["--reporter", "dot"]);
    assert!(r.stdout.starts_with("x.!\n"), "{}", r.all());
    assert!(
        r.stdout.contains(FOOTER) && r.stdout.contains(SUMMARY),
        "{}",
        r.all()
    );

    let r = run(&p, &["--reporter", "verbose"]);
    let out = &r.stdout;
    let a = out
        .find(&format!("FAIL  {}", String::from("a.test.ts")))
        .expect(out);
    let adds = out.find("\n   ✓ adds <&> (").expect(out);
    let later = out.find("\n   - later (").expect(out);
    let breaks = out.find("\n   ✗ breaks (").expect(out);
    let b = out
        .find(&format!("PASS  {}", String::from("b.test.ts")))
        .expect(out);
    let ok = out.find("\n   ✓ ok (").expect(out);
    let c = out
        .find(&format!("ERROR {}", String::from("c.test.ts")))
        .expect(out);
    assert!(
        a < adds && adds < later && later < breaks && breaks < b && b < ok && ok < c,
        "{out}"
    );
    assert!(out.contains(FOOTER), "{out}");
}

#[test]
fn json_reporter_keeps_stdout_clean() {
    let p = fixture("rep-json");
    let r = run(&p, &["--reporter", "json"]);
    let j = json_report(&r);
    assert_eq!(j["numTotalTestSuites"], 3);
    assert_eq!(j["numTotalTests"], 4);
    assert_eq!(j["numPassedTests"], 2);
    assert_eq!(j["numFailedTests"], 1);
    assert_eq!(j["numPendingTests"], 1);
    assert_eq!(j["success"], false);
    let st: Vec<_> = j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["status"].as_str().unwrap().to_string())
        .collect();
    // a load error is a failed file with the error's message (vitest's shape)
    assert_eq!(st, ["failed", "passed", "failed"]);
    assert_eq!(j["testResults"][2]["message"], "top <boom>");
    assert_eq!(j["testResults"][2]["name"], path(&p, "c.test.ts"));
    assert!(r.stderr.contains(SUMMARY), "{}", r.all());
    assert!(!r.stdout.contains("Test Files"), "{}", r.all());
}

#[test]
fn test_console_output_never_corrupts_a_stdout_artifact() {
    // turbo-test#32 item 6: `[js] [log] …` forwarding went to stdout ahead of the JSON/XML/TAP
    // artifact, so `JSON.parse(stdout)` failed. With an artifact on stdout it goes to stderr.
    let p = Project::new("rep-console");
    p.file(
        "log.test.js",
        "test('log', () => { console.log('hello-from-test'); console.error('err-from-test') })\n",
    );
    for reporter in ["json", "tap"] {
        let r = turbo_test(&p.dir, &["--reporter", reporter, "log.test.js"], &[]);
        assert_eq!(r.code, Some(0), "{reporter}: {}", r.all());
        assert!(!r.stdout.contains("[js]"), "{reporter}: {}", r.all());
        assert!(
            r.stderr.contains("[js] [log] hello-from-test\n")
                && r.stderr.contains("[js] [error] err-from-test\n"),
            "{reporter}: {}",
            r.all()
        );
    }
    let r = turbo_test(&p.dir, &["--reporter", "json", "log.test.js"], &[]);
    assert_eq!(json_report(&r)["numPassedTests"], 1);
    // junit keeps each test's output in its test case (vitest, turbo-test#81)
    let r = turbo_test(&p.dir, &["--reporter", "junit", "log.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("            <system-out>\nhello-from-test\n\n            </system-out>\n            <system-err>\nerr-from-test\n\n            </system-err>\n"),
        "{}",
        r.all()
    );
    assert!(!r.all().contains("[js]"), "{}", r.all());
    // human reporters, and a json artifact written to --outputFile, keep the console on stdout
    for args in [
        &["log.test.js"][..],
        &["--reporter", "verbose", "log.test.js"],
        &[
            "--reporter",
            "json",
            "--outputFile",
            "r.json",
            "log.test.js",
        ],
    ] {
        let r = turbo_test(&p.dir, args, &[]);
        assert_eq!(r.code, Some(0), "{args:?}: {}", r.all());
        assert!(
            r.stdout.contains("[js] [log] hello-from-test\n"),
            "{args:?}: {}",
            r.all()
        );
        assert!(!r.stderr.contains("[js]"), "{args:?}: {}", r.all());
    }
}

#[test]
fn file_without_tests_fails_the_run() {
    // turbo-test#32 item 7: a file that collects no test printed FAIL but counted as passed and
    // exited 0. vitest fails it with "No test suite found in file <path>" unless
    // --passWithNoTests (vitest runner `runFiles`: `!file.tasks.length && !passWithNoTests`).
    let p = Project::new("rep-empty");
    p.file("empty.spec.js", "export const x = 1;\n");
    p.file("a.test.js", "test('a', () => {})\n");
    let (e, a) = (path(&p, "empty.spec.js"), path(&p, "a.test.js"));
    let msg = format!("No test suite found in file {e}");
    let r = turbo_test(&p.dir, &["empty.spec.js", "a.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains(&format!(
            "PASS  a.test.js  (1 passed, 0 failed)\nERROR empty.spec.js  ({msg})"
        )),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("Test Files  1 failed | 1 passed (2)"),
        "{}",
        r.all()
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", &e, &a], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    let j = json_report(&r);
    assert_eq!(j["success"], false);
    // the files run in discovery order (positional arguments are filters, turbo-test#58)
    assert_eq!(j["testResults"][0]["status"], "passed");
    assert_eq!(common::file_status(&j["testResults"][1]), "error");
    let r = turbo_test(&p.dir, &["--reporter", "junit", &e], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains(&format!(
            "        <testcase classname=\"empty.spec.js\" name=\"empty.spec.js\" time=\"0\">\n            <failure message=\"{msg}\" type=\"Error\">\nError: {msg}\n            </failure>\n"
        )),
        "{}",
        r.all()
    );
    let r = turbo_test(&p.dir, &["--reporter", "tap", &e], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        r.stdout,
        format!(
            "TAP version 13\n1..1\nnot ok 1 - empty.spec.js\n  ---\n  message: {msg}\n  ...\n\n"
        ),
        "{}",
        r.all()
    );
    // --passWithNoTests also covers an empty file (vitest semantics): it passes
    let r = turbo_test(
        &p.dir,
        &["--passWithNoTests", "empty.spec.js", "a.test.js"],
        &[],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout
            .contains("PASS  empty.spec.js  (0 passed, 0 failed)"),
        "{}",
        r.all()
    );
}

#[test]
fn skipped_or_filtered_files_are_not_empty() {
    // Collected-but-not-run tests are tasks too (vitest only errors on a file with NO tasks):
    // skipped/todo tests and suites, and tests a -t pattern filters out, keep the file green.
    let p = Project::new("rep-skipped");
    p.file("skip.test.js", "test.skip('s', () => {})\n");
    p.file("todo.test.js", "test.todo('t')\n");
    p.file(
        "dskip.test.js",
        "describe.skip('d', () => { test('x', () => {}) })\n",
    );
    p.file("dtodo.test.js", "describe.todo('d')\n");
    p.file(
        "dif.test.js",
        "describe.skipIf(true)('d', () => {})\ndescribe.runIf(false)('e', () => {})\n",
    );
    p.file("a.test.js", "test('alpha', () => {})\n");
    p.file("empty.test.js", "export {}\n");
    let files = [
        "skip.test.js",
        "todo.test.js",
        "dskip.test.js",
        "dtodo.test.js",
        "dif.test.js",
    ];
    let r = turbo_test(&p.dir, &files, &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    // vitest 4.1.12: files none of whose tasks run are skipped files (turbo-test#78)
    assert!(
        r.stdout
            .contains("\n Test Files  5 skipped (5)\n      Tests  2 skipped | 1 todo (3)\n"),
        "{}",
        r.all()
    );
    for f in files {
        assert!(
            r.stdout
                .contains(&format!("PASS  {f}  (0 passed, 0 failed)")),
            "{f}: {}",
            r.all()
        );
    }
    let r = turbo_test(&p.dir, &["-t", "nomatch", "a.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    // under isolate reuse the skipped-suite marker is reset per file: a skipped-suite file
    // followed by an empty one on the same worker still fails the empty one
    let r = turbo_test(
        &p.dir,
        &[
            "--no-isolate",
            "--jobs",
            "1",
            "dskip.test.js",
            "empty.test.js",
        ],
        &[],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("PASS  dskip.test.js")
            && r.stdout
                .contains("ERROR empty.test.js  (No test suite found in file "),
        "{}",
        r.all()
    );
}

#[test]
fn junit_reporter() {
    // vitest 4.1.12's report for the same files (with `globals: true`), normalised
    // (turbo-test#81): the skipped test has a case of its own, the failure its printed error
    let p = fixture("rep-junit");
    let r = run(&p, &["--reporter", "junit"]);
    assert_eq!(
        normalize_junit(&r.stdout),
        include_str!("data/junit-fixture-vitest-4.1.12.xml"),
        "{}",
        r.all()
    );
    assert!(r.stderr.contains(SUMMARY), "{}", r.all());
}

#[test]
fn tap_reporter() {
    let p = fixture("rep-tap");
    let r = run(&p, &["--reporter", "tap"]);
    // the load-error file is a `not ok` point too (turbo-test#32 item 4), never dropped
    assert_eq!(
        r.stdout,
        format!(
            "TAP version 13\n1..5\nok 1 - adds <&>\nok 2 - later # SKIP\nnot ok 3 - breaks\n  ---\n  message: expected 'x' to be 'y' // Object.is equality\n  ...\nok 4 - ok\nnot ok 5 - {}\n  ---\n  message: module errored: Error: top <boom>\n  ...\n\n",
            String::from("c.test.ts")
        ),
        "{}",
        r.all()
    );
    assert!(r.stderr.contains(SUMMARY), "{}", r.all());
}

#[test]
fn output_file_failures_and_missing_dirs() {
    // turbo-test#32 item 5: text reporter + --outputFile into a missing dir silently wrote
    // nothing (exit 0); an unwritable --outputFile never failed the run.
    let p = Project::new("rep-outfile-dirs");
    p.file("a.test.js", "test('a', () => {})\n");
    let target = path(&p, "no/such/dir/out.txt");
    let r = turbo_test(&p.dir, &["--outputFile", &target, "a.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "PASS  a.test.js  (1 passed, 0 failed)\n"
    );
    std::fs::create_dir_all(p.path("adir")).unwrap();
    for reporter in ["default", "json"] {
        let r = turbo_test(
            &p.dir,
            &["--reporter", reporter, "--outputFile", "adir", "a.test.js"],
            &[],
        );
        assert_eq!(r.code, Some(1), "{reporter}: {}", r.all());
        assert!(
            r.stderr
                .contains("turbo-test: failed to write --outputFile 'adir': "),
            "{reporter}: {}",
            r.all()
        );
    }
}

#[test]
fn output_file_artifacts() {
    let p = fixture("rep-outfile");
    // artifact → file (parent dirs created); stdout says where it went, as vitest's
    let r = run(
        &p,
        &["--reporter", "junit", "--outputFile", "out/deep/junit.xml"],
    );
    let xml = std::fs::read_to_string(p.path("out/deep/junit.xml")).unwrap();
    assert!(
        xml.starts_with("<?xml") && xml.ends_with("</testsuites>\n"),
        "{xml}"
    );
    assert_eq!(
        r.stdout,
        format!(
            "JUNIT report written to {}\n",
            path(&p, "out/deep/junit.xml")
        ),
        "{}",
        r.all()
    );
    assert!(r.stderr.contains(SUMMARY), "{}", r.all());

    run(&p, &["--reporter", "tap", "--output-file", "r.tap"]);
    assert!(std::fs::read_to_string(p.path("r.tap"))
        .unwrap()
        .starts_with("TAP version 13\n1..5\n"));

    let r = run(&p, &["--reporter", "json", "--outputFile", "r.json"]);
    assert_eq!(
        r.stdout,
        format!("JSON report written to {}\n", path(&p, "r.json"))
    );
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(p.path("r.json")).unwrap()).unwrap();
    assert_eq!(j["numFailedTests"], 1);

    // human reporter + --outputFile → the PASS/FAIL lines are written, verbose still prints
    let r = run(&p, &["--reporter", "verbose", "--outputFile", "lines.txt"]);
    let txt = std::fs::read_to_string(p.path("lines.txt")).unwrap();
    assert!(
        txt.starts_with(&format!(
            "FAIL  {}  (1 passed, 1 failed)\n        ✗ breaks",
            String::from("a.test.ts")
        )),
        "{txt}"
    );
    assert!(
        txt.ends_with(&format!(
            "ERROR {}  (module errored: Error: top <boom>)\n",
            String::from("c.test.ts")
        )),
        "{txt}"
    );
    assert!(r.stdout.contains("   ✓ adds <&> ("), "{}", r.all());

    // unwritable target (an existing directory) → error on stderr, run still reported
    std::fs::create_dir_all(p.path("adir")).unwrap();
    let r = run(&p, &["--reporter", "json", "--outputFile", "adir"]);
    assert!(
        r.stderr
            .contains("turbo-test: failed to write --outputFile 'adir': "),
        "{}",
        r.all()
    );
    assert_eq!(r.stdout, "", "{}", r.all());
}

#[test]
fn the_footer_counts_as_vitest_get_state_string() {
    // turbo-test#78: vitest 4.1.12's summary (`getStateString`): `failed | passed | expected fail |
    // skipped | todo (total)` with the zero parts left out, `no tests` when there is none, and a
    // file none of whose tests ran counted `skipped`. Every line is vitest's for the same files.
    let p = Project::new("rep-footer");
    p.file(
        "m.test.js",
        "test('a', () => {})\ntest.skip('b', () => {})\ntest.todo('c')\ntest.fails('f', () => { throw 1 })\n",
    );
    p.file("s.test.js", "test.skip('b', () => {})\n");
    p.file("t.test.js", "test.todo('t')\n");
    p.file(
        "f.test.js",
        "test('x', () => { throw new Error('no') })\ntest.fails('g', () => {})\n",
    );
    p.file("l.test.js", "import './missing.js'\ntest('x', () => {})\n");
    let cases: [(&[&str], &str, &str); 6] = [
        (
            &["m.test.js"],
            "1 passed (1)",
            "1 passed | 1 expected fail | 1 skipped | 1 todo (4)",
        ),
        (&["s.test.js"], "1 skipped (1)", "1 skipped (1)"),
        (&["t.test.js"], "1 skipped (1)", "1 todo (1)"),
        (&["f.test.js"], "1 failed (1)", "2 failed (2)"),
        (&["l.test.js"], "1 failed (1)", "no tests"),
        (
            &[
                "m.test.js",
                "s.test.js",
                "t.test.js",
                "f.test.js",
                "l.test.js",
            ],
            "2 failed | 1 passed | 2 skipped (5)",
            "2 failed | 1 passed | 1 expected fail | 2 skipped | 2 todo (8)",
        ),
    ];
    for (files, test_files, tests) in cases {
        let r = turbo_test(&p.dir, files, &[]);
        let want = format!("\n Test Files  {test_files}\n      Tests  {tests}\n   Duration  ");
        assert!(
            r.stdout.contains(&want),
            "{files:?}: want {want:?}\n{}",
            r.all()
        );
    }
}

/// The project behind `tests/data/junit-vitest-4.1.12.xml`: nested describes, a passing, a
/// failing (number and object diffs) and a throwing test, skip / todo, console output, XML
/// specials, a failing `beforeAll`, a file that fails to load and a skipped-only file.
const JUNIT_FILES: [(&str, &str); 5] = [
    ("package.json", "{\"type\":\"module\"}\n"),
    (
        "a.test.ts",
        "import { describe, expect, it } from 'vitest';\n\
         describe('outer', () => {\n\
         \x20 it('passes', () => { expect(1).toBe(1); });\n\
         \x20 describe('inner', () => {\n\
         \x20   it('fails', () => { expect(1).toBe(2); });\n\
         \x20   it.skip('skipped', () => {});\n\
         \x20 });\n\
         \x20 it.todo('todo');\n\
         });\n\
         it('objects', () => { expect({ a: 1, b: [1, 2] }).toEqual({ a: 2, b: [1, 2] }); });\n\
         it('throws', () => { throw new TypeError('bad <thing> & \"x\"'); });\n\
         it('top <&> \"q\"', () => { console.log('out line'); console.error('err line'); console.log('two', { a: 1 }); });\n",
    ),
    (
        "b.test.ts",
        "import { beforeAll, describe, it } from 'vitest';\n\
         describe('hooked', () => {\n\
         \x20 beforeAll(() => { throw new Error('hook boom'); });\n\
         \x20 it('never', () => {});\n\
         });\n\
         it('b ok', () => {});\n",
    ),
    ("c.test.ts", "throw new Error('load boom');\n"),
    (
        "d.test.ts",
        "import { it } from 'vitest';\nit.skip('only skipped', () => {});\n",
    ),
];

/// A JUnit report with its run-dependent attributes (`time`, `timestamp`, `hostname`) blanked.
fn normalize_junit(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(at) = ["time=\"", "timestamp=\"", "hostname=\""]
        .iter()
        .filter_map(|k| rest.find(&format!(" {k}")).map(|i| (i, *k)))
        .min()
    {
        let (i, key) = at;
        let start = i + 1 + key.len();
        let end = start + rest[start..].find('"').unwrap();
        out.push_str(&rest[..start]);
        out.push_str(match key {
            "time=\"" => "T",
            "timestamp=\"" => "S",
            _ => "H",
        });
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// turbo-test#81: `--reporter junit` writes vitest 4.1.12's report byte for byte (once the
/// timings, timestamps and host name are normalised) on both loaders, to stdout or to
/// `--outputFile` (then stdout says where); the tests' console output is in the report only.
#[test]
fn junit_is_vitests_report() {
    let expected = include_str!("data/junit-vitest-4.1.12.xml");
    for esbuild in [false, true] {
        let p = Project::new("rep-junit-vitest");
        if esbuild {
            p.with_esbuild();
        }
        for (path, src) in JUNIT_FILES {
            p.file(path, src);
        }
        let r = turbo_test(&p.dir, &["--reporter", "junit"], &[]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert_eq!(
            normalize_junit(&r.stdout),
            expected,
            "esbuild={esbuild}\n{}",
            r.all()
        );
        // stderr: only turbo-test's own summary line (no test console output)
        assert!(
            r.stderr.trim().starts_with("4 files | ") && r.stderr.trim().lines().count() == 1,
            "esbuild={esbuild}\n{}",
            r.stderr
        );
        let r = turbo_test(
            &p.dir,
            &["--reporter", "junit", "--outputFile", "out/r.xml"],
            &[],
        );
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert_eq!(
            r.stdout,
            format!("JUNIT report written to {}\n", path(&p, "out/r.xml"))
        );
        let file = std::fs::read_to_string(p.path("out/r.xml")).unwrap();
        assert_eq!(normalize_junit(&file), expected, "esbuild={esbuild}");
    }
}

/// turbo-test#96: every test function gets vitest's TestContext — `task` (its name, ids, suite,
/// file, result), `skip(note?)` / `skip(condition, note?)` (skipped at run time, even when the
/// PendingError is caught), `onTestFinished` (after afterEach, last registered first) /
/// `onTestFailed`, `signal`, `expect`, `annotate` (junit `<properties>`); beforeEach / afterEach
/// get it too, `.for` rows and `it.extend` fixtures see the same context, `.each` handlers do
/// not. `tests/data/junit-context-vitest-4.1.12.xml` is vitest 4.1.12's report (the tests log
/// what they see), on both loaders.
#[test]
fn test_functions_get_vitests_test_context() {
    for esbuild in [false, true] {
        let p = Project::new("rep-junit-context");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", "{\"type\":\"module\"}\n");
        p.file("a.test.ts", include_str!("data/context/a.test.ts"));
        let r = turbo_test(&p.dir, &["--reporter", "junit"], &[]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert_eq!(
            normalize_junit(&r.stdout),
            include_str!("data/junit-context-vitest-4.1.12.xml"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}

/// turbo-test#97: what is not an `Error` with a message prints as vitest prints it — a thrown
/// primitive is `Unknown Error: <value>` (junit's `<failure>` with its `message` only), a value
/// without a message its serialized form (`{ a: 1, stacks: [] }`), and the serialized error's own
/// properties (inherited methods as `Function<name>`; a cause's too) follow its frames as
/// `Serialized Error: { … }`; `tests/data/junit-errors-vitest-4.1.12.xml` is vitest 4.1.12's
/// report, on both loaders. The JSON reporter's `failureMessages` are the serialized error's
/// stack, else its message, else null.
#[test]
fn thrown_values_and_error_properties_print_as_vitest_prints_them() {
    for esbuild in [false, true] {
        let p = Project::new("rep-junit-errors");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", "{\"type\":\"module\"}\n");
        p.file("a.test.ts", include_str!("data/errors/a.test.ts"));
        p.file("b.test.ts", include_str!("data/errors/b.test.ts"));
        let r = turbo_test(&p.dir, &["--reporter", "junit"], &[]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert_eq!(
            normalize_junit(&r.stdout),
            include_str!("data/junit-errors-vitest-4.1.12.xml"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
        let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
        let v = json_report(&r);
        let mut files: Vec<&serde_json::Value> =
            v["testResults"].as_array().unwrap().iter().collect();
        files.sort_by_key(|f| f["name"].as_str().unwrap().to_string());
        let firsts: Vec<String> = files
            .iter()
            .flat_map(|f| f["assertionResults"].as_array().unwrap())
            .map(|a| {
                let first = match &a["failureMessages"][0] {
                    serde_json::Value::String(s) => format!("{:?}", s.split('\n').next().unwrap()),
                    other => other.to_string(),
                };
                format!("{} {first}", a["title"].as_str().unwrap())
            })
            .collect();
        assert_eq!(
            firsts,
            [
                "string \"plain string\"",
                "props \"Error: with code\"",
                "object null",
                "null \"null\"",
                "multi \"line one\"",
                "undef \"undefined\"",
                "number \"42\"",
                "empty \"\"",
                "named \"msg\"",
                "nameonly null",
                "class \"MyError: mine\"",
                "cause props \"Error: outer\"",
                "fn prop \"TypeError: typed\"",
            ],
            "esbuild={esbuild}"
        );
    }
}

/// turbo-test#81, the failure bodies: string / multi-line / type-mismatch / empty diffs, no diff
/// for the comparison matchers (vitest passes showDiff false), a helper module's frame with its
/// function name (mapped back through that module's transform), an error's cause, and console
/// output split where a macrotask came between writes — `tests/data/junit-frames-vitest-4.1.12.xml`
/// is vitest 4.1.12's report for these files, on both loaders.
#[test]
fn junit_failures_print_as_vitest_prints_them() {
    let a = "import { expect, it } from 'vitest';\n\
             import { helper } from './h';\n\
             it('str', () => { expect('x').toBe('y'); });\n\
             it('multi', () => { expect('a\\nb').toBe('a\\nc'); });\n\
             it('helper', () => { helper(); });\n\
             it('cause', () => { throw new Error('outer', { cause: new TypeError('inner') }); });\n\
             it('noshow', () => { expect([1]).toHaveLength(2); });\n\
             it('async', async () => { await Promise.resolve(); console.log('after await'); await new Promise((r) => setTimeout(r, 5)); console.log('after timer'); });\n";
    let b = "import { expect, it } from 'vitest';\n\
             it('types', () => { expect(1).toEqual('1'); });\n\
             it('empty', () => { expect('').toBe('x'); });\n\
             it('quotes', () => { expect('a\"b').toBe('a\"c'); });\n\
             it('nodiff', () => { expect(1).toBeGreaterThan(2); });\n";
    for esbuild in [false, true] {
        let p = Project::new("rep-junit-frames");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", "{\"type\":\"module\"}\n");
        p.file("a.test.ts", a);
        p.file("b.test.ts", b);
        p.file(
            "h.ts",
            "export function helper() {\n  throw new RangeError('from helper');\n}\n",
        );
        let r = turbo_test(&p.dir, &["--reporter", "junit"], &[]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert_eq!(
            normalize_junit(&r.stdout),
            include_str!("data/junit-frames-vitest-4.1.12.xml"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}

/// turbo-test#99: a test file that fails to compile (itself, or a module it imports) fails with
/// V8's SyntaxError as vitest 4.1.12 reports it (`Failed Suites`, the json `message`, the junit
/// failing case); turbo-test said `graph load failed` and V8 printed `Uncaught SyntaxError` to
/// stdout three times, with a line of the transformed code. vitest prints the error once for the
/// files whose import of one module failed (b, c), and apart for a's own.
#[test]
fn a_compile_error_is_the_files_syntax_error() {
    const MSG: &str = "Invalid regular expression: /e{,2}/u: Incomplete quantifier";
    let case = |f: &str| {
        format!(
            "    <testsuite name=\"{f}\" timestamp=\"S\" hostname=\"H\" tests=\"1\" failures=\"1\" errors=\"0\" skipped=\"0\" time=\"T\">\n        <testcase classname=\"{f}\" name=\"{f}\" time=\"T\">\n            <failure message=\"{MSG}\" type=\"SyntaxError\">\nSyntaxError: {MSG}\n            </failure>\n        </testcase>\n    </testsuite>\n"
        )
    };
    // vitest 4.1.12's reports for this project
    let junit = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<testsuites name=\"vitest tests\" tests=\"3\" failures=\"3\" errors=\"0\" time=\"T\">\n{}{}{}</testsuites>\n",
        case("a.test.ts"),
        case("b.test.ts"),
        case("c.test.ts")
    );
    let line = "⎯".repeat(24);
    let suites = format!(
        "\n⎯⎯⎯⎯⎯⎯ Failed Suites 3 ⎯⎯⎯⎯⎯⎯⎯\n\n FAIL  a.test.ts [ a.test.ts ]\nSyntaxError: {MSG}\n{line}[1/3]⎯\n\n FAIL  b.test.ts [ b.test.ts ]\n FAIL  c.test.ts [ c.test.ts ]\nSyntaxError: {MSG}\n{line}[2/3]⎯\n\n"
    );
    for esbuild in [false, true] {
        let p = Project::new("rep-compile-error");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", "{\"type\":\"module\"}\n");
        p.file(
            "a.test.ts",
            "import { it } from 'vitest';\nit('x', () => { console.log(/e{,2}/u); });\n",
        );
        let b = "import { it } from 'vitest';\nimport { v } from './bad';\nit('y', () => { console.log(v); });\n";
        p.file("b.test.ts", b);
        p.file("c.test.ts", b);
        p.file(
            "bad.ts",
            "export const v = 1;\nexport const w = /e{,2}/u;\n",
        );
        let r = run(&p, &[]);
        assert!(
            r.stdout.starts_with(&format!(
                "ERROR a.test.ts  (SyntaxError: {MSG})\nERROR b.test.ts  (SyntaxError: {MSG})\nERROR c.test.ts  (SyntaxError: {MSG})\n"
            )),
            "esbuild={esbuild}\n{}",
            r.all()
        );
        assert!(
            !r.all().contains("Uncaught"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
        assert!(!r.all().contains("graph load failed"), "{}", r.all());
        let r = turbo_test(&p.dir, &[], &[("TURBO_LEGACY_REPORTER", "0")]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert!(
            r.stderr.ends_with(&suites),
            "esbuild={esbuild}\n{}",
            r.all()
        );
        assert!(
            !r.stdout.contains("Uncaught"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
        let j = json_report(&run(&p, &["--reporter", "json"]));
        let msgs: Vec<&str> = j["testResults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["message"].as_str().unwrap())
            .collect();
        assert_eq!(msgs, [MSG, MSG, MSG], "esbuild={esbuild}");
        let r = run(&p, &["--reporter", "junit"]);
        assert_eq!(
            normalize_junit(&r.stdout),
            junit,
            "esbuild={esbuild}\n{}",
            r.all()
        );
        assert!(
            !r.all().contains("Uncaught"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}
