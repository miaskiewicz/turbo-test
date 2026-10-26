//! The human reporters against vitest 4.1.12 (turbo-test#94): `--reporter default`,
//! `--reporter verbose` and, when an AI agent runs the tests (std-env `isAgent`), the agent
//! reporter vitest picks without `--reporter`. `tests/data/human/<scenario>-<reporter>.{out,err}`
//! are vitest's stdout / stderr for each scenario's project with `NO_COLOR=1` (TZ=UTC), the root,
//! the start time, the duration line and per-line durations normalised.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// Each scenario's files (a `package.json` with `"type": "module"` is added).
const SCENARIOS: [(&str, &[(&str, &str)]); 6] = [
    (
        "s1",
        &[(
            "a.test.ts",
            "import { expect, it } from 'vitest';\nit('one', () => { expect(1).toBe(1); });\nit('two', () => {});\n",
        )],
    ),
    (
        "s2",
        &[(
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
        )],
    ),
    (
        "s3",
        &[(
            "b.test.ts",
            "import { beforeAll, describe, it } from 'vitest';\n\
             describe('hooked', () => {\n\
             \x20 beforeAll(() => { throw new Error('hook boom'); });\n\
             \x20 it('never', () => {});\n\
             });\n\
             it('b ok', () => {});\n",
        )],
    ),
    ("s4", &[("c.test.ts", "throw new Error('load boom');\n")]),
    (
        "s5",
        &[(
            "a.test.ts",
            "import { beforeAll, describe, expect, it } from 'vitest';\n\
             console.log('top level');\n\
             describe('d', () => {\n\
             \x20 beforeAll(() => { console.log('in hook'); });\n\
             \x20 it('t', () => { console.error('in test'); expect(1).toBe(1); });\n\
             });\n",
        )],
    ),
    (
        "s6",
        &[(
            "a.test.ts",
            "import { expect, it } from 'vitest';\nit('snap', () => { expect({ a: 1 }).toMatchSnapshot(); });\n",
        )],
    ),
];

const SNAP: &str = "// Vitest Snapshot v1, https://vitest.dev/guide/snapshot.html\n\n\
                    exports[`gone 1`] = `2`;\n\nexports[`snap 1`] = `1`;\n";
const OLD_SNAP: &str = "// Vitest Snapshot v1, https://vitest.dev/guide/snapshot.html\n\n\
                        exports[`old 1`] = `1`;\n";
/// Two files, one with an obsolete snapshot and one whose snapshot file has no snapshot left (the
/// larger file first: vitest's sequencer runs files by size under --no-file-parallelism).
const OBSOLETE: &[(&str, &str)] = &[
    (
        "a.test.ts",
        "import { expect, it } from 'vitest';\n\
         // the larger file: vitest's sequencer runs it first\n\
         it('snap', () => { expect(1).toMatchSnapshot(); });\n",
    ),
    ("__snapshots__/a.test.ts.snap", SNAP),
    (
        "b.test.ts",
        "import { it } from 'vitest';\nit('plain', () => {});\n",
    ),
    ("__snapshots__/b.test.ts.snap", OLD_SNAP),
];

/// More scenarios: a slow test (over vitest's 300ms slowTestThreshold) in a passing and in a
/// failed file, retried tests, obsolete snapshots and, with `-u`, removed ones, identical errors
/// merged, console output per task. Each with the extra arguments both runners get.
/// A scenario: its name, its files and the arguments both runners get.
type Scenario = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static [&'static str],
);

const MORE: [Scenario; 7] = [
    (
        "s7",
        &[(
            "a.test.ts",
            "import { it } from 'vitest';\n\
             it('slow', async () => { await new Promise(r => setTimeout(r, 320)); });\n\
             it('\\ttab', () => {});\n",
        )],
        &[],
    ),
    (
        "s8",
        &[(
            "a.test.ts",
            "import { describe, it, expect } from 'vitest';\n\
             describe('d', () => {\n\
             \x20 it('slow', async () => { await new Promise(r => setTimeout(r, 320)); });\n\
             \x20 it('f', () => { expect(1).toBe(2); });\n\
             });\n",
        )],
        &[],
    ),
    (
        "s9",
        &[(
            "a.test.ts",
            "import { it } from 'vitest';\n\
             let n = 0;\n\
             let m = 0;\n\
             it('flaky', { retry: 2 }, () => { n++; if (n < 2) throw new Error('once'); });\n\
             it('always', { retry: 1 }, () => { throw new Error('nope ' + (++m)); });\n",
        )],
        &[],
    ),
    ("s10", OBSOLETE, &["--no-file-parallelism"]),
    ("s11", OBSOLETE, &["--no-file-parallelism", "-u"]),
    // two tests failing with the same stack: one error under both FAIL lines
    (
        "s12",
        &[(
            "a.test.ts",
            "import { expect, it } from 'vitest';\n\
             it.each([1, 2])('row %i', () => { expect(1).toBe(2); });\n\
             it('other', () => { expect(1).toBe(2); });\n",
        )],
        &[],
    ),
    // console output of the file, its suites' hooks and its tests: the agent reporter prints
    // that of failed tasks only, each when its task finishes
    (
        "s14",
        &[(
            "a.test.ts",
            "import { beforeAll, describe, expect, it } from 'vitest';\n\
             console.log('file level');\n\
             describe('outer', () => {\n\
             \x20 beforeAll(() => { console.log('outer hook'); });\n\
             \x20 describe('inner', () => {\n\
             \x20   beforeAll(() => { console.log('inner hook'); });\n\
             \x20   it('fails', () => { console.log('in failing'); expect(1).toBe(2); });\n\
             \x20 });\n\
             \x20 describe('calm', () => {\n\
             \x20   beforeAll(() => { console.log('calm hook'); });\n\
             \x20   it('passes', () => { console.log('in passing'); });\n\
             \x20 });\n\
             });\n",
        )],
        &[],
    ),
];

/// vitest's reference output for `scenario` and `reporter` (`.out` or `.err`).
fn reference(scenario: &str, reporter: &str, stream: &str) -> String {
    let path = common::repo_root()
        .join("tests/data/human")
        .join(format!("{scenario}-{reporter}.{stream}"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `text` with the run-dependent parts as the reference files spell them: the project root,
/// the start time, the duration line and every duration (` 12ms` before a space or the line end).
fn normalize(text: &str, root: &str) -> String {
    text.replace(root, "<root>")
        .lines()
        .map(|l| {
            if l.starts_with("   Start at  ") {
                return "   Start at  <time>".to_string();
            }
            if l.starts_with("   Duration  ") {
                return "   Duration  <duration>".to_string();
            }
            let words: Vec<&str> = l.split(' ').collect();
            words
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let ms = w.strip_suffix("ms").filter(|d| {
                        i > 0 && !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
                    });
                    if ms.is_some() {
                        "<ms>"
                    } else {
                        w
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if text.ends_with('\n') { "\n" } else { "" }
}

fn project(name: &str, files: &[(&str, &str)], esbuild: bool) -> Project {
    let p = Project::new(name);
    if esbuild {
        p.with_esbuild();
    }
    p.file("package.json", "{\"type\":\"module\"}\n");
    for (path, src) in files {
        p.file(path, src);
    }
    p
}

/// Plain output (`NO_COLOR`), vitest's human reporters: the RUN banner, console blocks, module
/// and test lines, the Failed Suites / Failed Tests sections with code frames on stderr, the
/// footer with `Start at` and the duration breakdown.
#[test]
fn human_reporters_print_what_vitest_prints() {
    for (scenario, files) in SCENARIOS {
        for reporter in ["default", "verbose"] {
            for esbuild in [false, true] {
                let p = project(&format!("hr-{scenario}-{reporter}"), files, esbuild);
                let root = p.dir.display().to_string();
                let r = turbo_test(
                    &p.dir,
                    &["--reporter", reporter],
                    &[("NO_COLOR", "1"), ("TURBO_LEGACY_REPORTER", "0")],
                );
                let ctx = format!("{scenario} {reporter} esbuild={esbuild}\n{}", r.all());
                assert_eq!(
                    normalize(&r.stdout, &root),
                    reference(scenario, reporter, "out"),
                    "{ctx}"
                );
                assert_eq!(
                    normalize(&r.stderr, &root),
                    reference(scenario, reporter, "err"),
                    "{ctx}"
                );
            }
        }
    }
}

/// Under an AI agent (std-env `isAgent`: `AI_AGENT`, `CLAUDECODE`, …) vitest's default is its
/// agent reporter: no colours, only failed modules and tests, console output of failed tasks only.
#[test]
fn an_agent_gets_vitests_agent_reporter() {
    for (scenario, files) in SCENARIOS {
        let p = project(&format!("hr-{scenario}-agent"), files, false);
        let root = p.dir.display().to_string();
        let r = turbo_test(
            &p.dir,
            &[],
            &[("AI_AGENT", "claude-code"), ("TURBO_LEGACY_REPORTER", "0")],
        );
        let ctx = format!("{scenario}\n{}", r.all());
        assert_eq!(
            normalize(&r.stdout, &root),
            reference(scenario, "agent", "out"),
            "{ctx}"
        );
        assert_eq!(
            normalize(&r.stderr, &root),
            reference(scenario, "agent", "err"),
            "{ctx}"
        );
    }
}

/// The scenarios of [`MORE`] with each human reporter (the agent one under `AI_AGENT`) on both
/// loaders, files run one at a time (`TURBO_JOBS=1`) in the order vitest's sequencer picks.
#[test]
fn slow_tests_retries_and_obsolete_snapshots_print_what_vitest_prints() {
    for (scenario, files, args) in MORE {
        for (reporter, esbuild) in [
            ("default", false),
            ("default", true),
            ("verbose", false),
            ("verbose", true),
            ("agent", false),
        ] {
            let p = project(&format!("hr-{scenario}-{reporter}"), files, esbuild);
            let root = p.dir.display().to_string();
            let mut argv: Vec<&str> = args.to_vec();
            let mut env = vec![("TURBO_LEGACY_REPORTER", "0"), ("TURBO_JOBS", "1")];
            if reporter == "agent" {
                env.push(("AI_AGENT", "claude-code"));
            } else {
                argv.extend(["--reporter", reporter]);
                env.push(("NO_COLOR", "1"));
            }
            let r = turbo_test(&p.dir, &argv, &env);
            let ctx = format!("{scenario} {reporter} esbuild={esbuild}\n{}", r.all());
            assert_eq!(
                normalize(&r.stdout, &root),
                reference(scenario, reporter, "out"),
                "{ctx}"
            );
            assert_eq!(
                normalize(&r.stderr, &root),
                reference(scenario, reporter, "err"),
                "{ctx}"
            );
        }
    }
}
