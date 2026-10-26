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

/// vitest's reference output for `scenario` and `reporter` (`.out` or `.err`).
fn reference(scenario: &str, reporter: &str, stream: &str) -> String {
    let path = common::repo_root()
        .join("tests/data/human")
        .join(format!("{scenario}-{reporter}.{stream}"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `text` with the run-dependent parts as the reference files spell them: the project root,
/// the start time, the duration line and the durations that end a line.
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
            match l.rsplit_once(' ') {
                Some((head, ms))
                    if ms.len() > 2
                        && ms.ends_with("ms")
                        && ms[..ms.len() - 2].bytes().all(|b| b.is_ascii_digit()) =>
                {
                    format!("{head} <ms>")
                }
                _ => l.to_string(),
            }
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
