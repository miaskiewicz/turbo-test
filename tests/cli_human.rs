//! The human reporters against vitest 4.1.12 (turbo-test#94): `--reporter default`,
//! `--reporter verbose` and, when an AI agent runs the tests (std-env `isAgent`), the agent
//! reporter vitest picks without `--reporter`. `tests/data/human/<scenario>-<reporter>.{out,err}`
//! are vitest's stdout / stderr for each scenario's project with `NO_COLOR=1` (TZ=UTC), the root,
//! the start time, the duration line and per-line durations normalised; `--reporter dot`
//! (turbo-test#100) against `tests/data/dot` and `tests/data/dot-color`.
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

/// A scenario: its name, its files and the arguments both runners get.
type Scenario = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static [&'static str],
);

/// More scenarios: a slow test (over vitest's 300ms slowTestThreshold) in a passing and in a
/// failed file, retried tests, obsolete snapshots and, with `-u`, removed ones, identical errors
/// merged, console output per task, diffs of strings and objects, console values (s18 and s19
/// below).
const MORE: [Scenario; 11] = [
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
    // the diffs vitest prints (and colours): short strings, lines, arrays, objects, numbers
    (
        "s16",
        &[(
            "a.test.ts",
            "import { expect, it } from 'vitest';\n\
             it('short strings', () => { expect('hello world').toBe('hello there'); });\n\
             it('lines', () => { expect('a\\nb\\nc').toBe('a\\nB\\nc'); });\n\
             it('arrays', () => { expect([1, 2, 3]).toEqual([1, 3]); });\n\
             it('nested', () => { expect({ a: 'x', n: { m: [true, null] } }).toEqual({ a: 'y', n: { m: [false, null] } }); });\n\
             it('numbers', () => { expect(0.1 + 0.2).toBe(0.3); });\n",
        )],
        &[],
    ),
    // console values, as Node's util.inspect prints (and colours) them
    (
        "s17",
        &[(
            "a.test.ts",
            "import { it } from 'vitest';\n\
             class K { constructor() { this.v = 1; } }\n\
             it('values', () => {\n\
             \x20 console.log(1, 'str', true, null, undefined, Symbol('s'), 10n);\n\
             \x20 console.log({ n: 1, s: 'x', b: false, u: undefined, nl: null, f() {}, arrow: () => 1, re: /a+/g, d: new Date(0), k: new K() });\n\
             \x20 console.log([1, 'a', [2, [3, [4, [5]]]]], new Map([['k', 1]]), new Set(['v']));\n\
             \x20 console.error('%s is %d', 'n', 42, { x: 1 });\n\
             });\n",
        )],
        &[],
    ),
    // more diffs: aligned multi-line strings, a long array (patch marks), values that serialize
    // the same, different types, trailing spaces, strings with nothing in common
    ("s18", &[("a.test.ts", S18)], &[]),
    // more console values: highlighted RegExps, circular and boxed values, a pending promise,
    // quoted and symbol keys, accessors, grouped arrays, multi-line strings, holes, a class
    ("s19", &[("a.test.ts", S19)], &[]),
];

const S18: &str = r#"import { expect, it } from 'vitest';
it('aligned', () => { expect('hello world\nsecond line\nthird').toBe('hello there\nsecond line\nthird!'); });
it('long', () => { const a = Array.from({ length: 30 }, (_, i) => i); const b = [...a]; b[15] = 'x'; expect(a).toEqual(b); });
it('json', () => { expect({ toJSON: () => 1, v: 1 }).toEqual({ toJSON: () => 1, v: 2 }); });
it('types', () => { expect(1).toEqual('1'); });
it('trailing', () => { expect('a  ').toBe('b  '); });
it('nothing common', () => { expect('abc').toBe('xyz'); });
"#;

const S19: &str = r#"import { it } from 'vitest';
it('more values', () => {
  console.log(/a(b|c)[^d-f]\d{2,3}?x*(?:y)(?=q)(?<n>z)\k<n>\u{41}\p{L}.^$|w{3,}/gu, /[\x41-\x5A]e{,2}(?<!x)(?!y)\1\x4/);
  const circ = { name: 'c' }; circ.self = circ;
  console.log(circ, new Uint8Array([1, 2]), new Number(3), new String('s'), new Promise(() => {}), new WeakMap());
  console.log({ 'quoted-key': 1, [Symbol('sk')]: 'v', get g() { return 1; }, set s(v) {}, arr: Array.from({ length: 12 }, (_, i) => i * 100) });
  console.log({ long: 'line one\nline two\nline three is a bit longer than the others to wrap' }, [ , 1, , ], class A {}, Symbol.iterator);
});
"#;

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

/// The colour vitest's getDurationPrefix gives a duration: yellow over its 300ms
/// slowTestThreshold, green otherwise.
const SLOW_MS: u64 = 300;

/// Coloured `text` as `tests/data/human-color` spells it (see [`reference_colours`]): the project
/// root, the `Start at` and `Duration` lines, and every duration's digits (` 12ms`,
/// ` 12\x1b[2mms`). A coloured duration must have its value's colour, then is spelled
/// `\x1b[3?m <n>` whatever the colour: a test fast in vitest's reference can be slow on a loaded
/// machine (turbo-test#109).
fn normalize_colored(text: &str, root: &str) -> String {
    text.replace(root, "<root>")
        .split('\n')
        .map(|l| {
            if l.contains("   Start at ") {
                return "<start>".to_string();
            }
            if l.contains("   Duration ") {
                return "<duration>".to_string();
            }
            let mut out = String::new();
            let mut rest = l;
            while let Some(i) = rest.find(|c: char| c.is_ascii_digit()) {
                let digits = rest[i..].len()
                    - rest[i..]
                        .trim_start_matches(|c: char| c.is_ascii_digit())
                        .len();
                let after = &rest[i + digits..];
                let duration = (after.starts_with("ms") || after.starts_with("\x1b[2mms"))
                    && rest[..i].ends_with(' ');
                // (a colour code's own digits went out already: look at what is written)
                out.push_str(&rest[..i]);
                let coloured =
                    duration && (out.ends_with("\x1b[32m ") || out.ends_with("\x1b[33m "));
                if coloured {
                    let value: u64 = rest[i..i + digits].parse().unwrap();
                    let yellow = out.ends_with("\x1b[33m ");
                    assert_eq!(
                        yellow,
                        value > SLOW_MS,
                        "{value}ms coloured {}: {l:?}",
                        if yellow { "yellow" } else { "green" }
                    );
                    out.truncate(out.len() - "\x1b[32m ".len());
                    out.push_str("\x1b[3?m ");
                }
                out.push_str(if duration {
                    "<n>"
                } else {
                    &rest[i..i + digits]
                });
                rest = after;
            }
            out.push_str(rest);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A coloured reference as [`normalize_colored`] spells its output: each duration's colour code is
/// `\x1b[3?m` (the colour itself is checked against the measured value instead).
fn reference_colours(text: String) -> String {
    text.replace("\x1b[32m <n>", "\x1b[3?m <n>")
        .replace("\x1b[33m <n>", "\x1b[3?m <n>")
}

/// turbo-test#109: a coloured duration is spelled `\x1b[3?m <n>` whatever its colour, once the
/// colour is checked against its value (vitest's getDurationPrefix: yellow over the 300ms
/// slowTestThreshold, green otherwise) — a test that is fast in vitest's reference can take
/// over 300ms on a loaded machine.
#[test]
fn coloured_durations_are_normalized_after_their_colour_is_checked() {
    let tail = "\x1b[2mms\x1b[22m\x1b[39m";
    for (n, colour) in [
        ("0", 32),
        ("12", 32),
        ("300", 32),
        ("301", 33),
        ("1412", 33),
    ] {
        assert_eq!(
            normalize_colored(&format!(" a.test.ts\x1b[{colour}m {n}{tail} 7 tests"), "/r"),
            format!(" a.test.ts\x1b[3?m <n>{tail} 7 tests"),
            "{n}"
        );
    }
    // a duration without a colour of its own (the plain reporters) keeps the rest as it is
    assert_eq!(
        normalize_colored("/r/a.ts 12ms 3", "/r"),
        "<root>/a.ts <n>ms 3"
    );
}

/// turbo-test#109: a duration in the wrong colour for its value is a failure, not normalized away.
#[test]
fn a_duration_coloured_against_the_threshold_fails() {
    for (n, colour) in [("300", 33), ("301", 32), ("5", 33)] {
        let r = std::panic::catch_unwind(|| {
            normalize_colored(&format!("\x1b[{colour}m {n}\x1b[2mms"), "/r")
        });
        assert!(r.is_err(), "{n} in {colour} was accepted");
    }
}

/// Without `NO_COLOR` (and no agent) vitest colours its output even in a pipe (tinyrainbow):
/// the labels, symbols, counts and durations, the console values (Node's util.inspect colours),
/// the diffs, the frames and the syntax-highlighted code frames (tinyhighlight). Every scenario
/// with both reporters on both loaders, against `tests/data/human-color`.
#[test]
fn coloured_output_is_vitests() {
    let all = SCENARIOS
        .iter()
        .map(|(name, files)| (*name, *files, &[][..]))
        .chain(MORE.iter().copied());
    for (scenario, files, args) in all {
        for reporter in ["default", "verbose"] {
            for esbuild in [false, true] {
                let p = project(&format!("hc-{scenario}-{reporter}"), files, esbuild);
                let root = p.dir.display().to_string();
                let mut argv: Vec<&str> = args.to_vec();
                argv.extend(["--reporter", reporter]);
                let env = [
                    ("TURBO_LEGACY_REPORTER", "0"),
                    ("TURBO_JOBS", "1"),
                    ("TERM", "xterm-256color"),
                ];
                let r = turbo_test(&p.dir, &argv, &env);
                let ctx = format!("{scenario} {reporter} esbuild={esbuild}\n{}", r.all());
                let reference = |stream: &str| {
                    let path = common::repo_root()
                        .join("tests/data/human-color")
                        .join(format!("{scenario}-{reporter}.{stream}"));
                    reference_colours(
                        std::fs::read_to_string(&path)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
                    )
                };
                assert_eq!(
                    normalize_colored(&r.stdout, &root),
                    reference("out"),
                    "{ctx}"
                );
                assert_eq!(
                    normalize_colored(&r.stderr, &root),
                    reference("err"),
                    "{ctx}"
                );
            }
        }
    }
}

/// `--reporter dot` against vitest 4.1.12's DotReporter (turbo-test#100): the RUN banner and
/// console blocks of the default reporter, then one character per test as it finishes (`·`
/// passed, `x` failed, `-` skipped or todo; off a TTY each is written on its own as
/// `formatTests([state])`, an empty yellow run before it), an empty line, the default footer, and
/// the default reporter's Failed Suites / Failed Tests sections on stderr. Every scenario, plain
/// (`NO_COLOR`, `tests/data/dot`) and coloured (`tests/data/dot-color`), on both loaders.
#[test]
fn dot_reporter_prints_what_vitest_prints() {
    let all = SCENARIOS
        .iter()
        .map(|(name, files)| (*name, *files, &[][..]))
        .chain(MORE.iter().copied());
    for (scenario, files, args) in all {
        for colored in [false, true] {
            for esbuild in [false, true] {
                let p = project(&format!("hd-{scenario}"), files, esbuild);
                let root = p.dir.display().to_string();
                let mut argv: Vec<&str> = args.to_vec();
                argv.extend(["--reporter", "dot"]);
                let mut env = vec![("TURBO_LEGACY_REPORTER", "0"), ("TURBO_JOBS", "1")];
                env.push(if colored {
                    ("TERM", "xterm-256color")
                } else {
                    ("NO_COLOR", "1")
                });
                let r = turbo_test(&p.dir, &argv, &env);
                let ctx = format!(
                    "{scenario} colored={colored} esbuild={esbuild}\n{}",
                    r.all()
                );
                let dir = if colored { "dot-color" } else { "dot" };
                let reference = |stream: &str| {
                    let path = common::repo_root()
                        .join("tests/data")
                        .join(dir)
                        .join(format!("{scenario}.{stream}"));
                    reference_colours(
                        std::fs::read_to_string(&path)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
                    )
                };
                let norm = |text: &str| {
                    if colored {
                        normalize_colored(text, &root)
                    } else {
                        normalize(text, &root)
                    }
                };
                assert_eq!(norm(&r.stdout), reference("out"), "{ctx}");
                assert_eq!(norm(&r.stderr), reference("err"), "{ctx}");
            }
        }
    }
}

/// turbo-test#96: a test skipped at run time (`ctx.skip(note?)`) shows its duration and note, and
/// `ctx.annotate()` calls print under the verbose test line and the failed test's error.
const CONTEXT: &[(&str, &str)] = &[(
    "a.test.ts",
    "import { describe, expect, it } from 'vitest';\n\
     it('noted', (ctx) => { ctx.skip('not today'); });\n\
     it('quiet', (ctx) => { ctx.skip(); });\n\
     it.skip('static', () => {});\n\
     describe('d', () => {\n\
     \x20 it('annotated', async ({ annotate }) => { await annotate('note one'); await annotate('note two'); await annotate('careful', 'warning'); });\n\
     \x20 it('fails', async ({ annotate }) => { await annotate('before failing'); expect(1).toBe(2); });\n\
     });\n",
)];

#[test]
fn skip_notes_and_annotations_print_what_vitest_prints() {
    for (reporter, esbuild) in [
        ("default", false),
        ("default", true),
        ("verbose", false),
        ("verbose", true),
        ("agent", false),
    ] {
        let p = project(&format!("hr-s20-{reporter}"), CONTEXT, esbuild);
        let root = p.dir.display().to_string();
        let (argv, env): (&[&str], _) = if reporter == "agent" {
            (
                &[],
                vec![("TURBO_LEGACY_REPORTER", "0"), ("AI_AGENT", "claude-code")],
            )
        } else {
            (
                &["--reporter", reporter],
                vec![("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
            )
        };
        let r = turbo_test(&p.dir, argv, &env);
        let ctx = format!("s20 {reporter} esbuild={esbuild}\n{}", r.all());
        assert_eq!(
            normalize(&r.stdout, &root),
            reference("s20", reporter, "out"),
            "{ctx}"
        );
        assert_eq!(
            normalize(&r.stderr, &root),
            reference("s20", reporter, "err"),
            "{ctx}"
        );
    }
}

/// turbo-test#97: thrown primitives (`Unknown Error: <value>`), values without a message (their
/// serialized form) and error properties (`Serialized Error: { … }`, verbose only, a cause's in
/// every reporter) in the Failed Tests section; s15 is the issue's repro, s21 more values.
#[test]
fn thrown_values_and_error_properties_print_what_vitest_prints() {
    for (scenario, file) in [
        ("s15", include_str!("data/errors/a.test.ts")),
        ("s21", include_str!("data/errors/b.test.ts")),
    ] {
        for (reporter, esbuild) in [
            ("default", false),
            ("verbose", false),
            ("verbose", true),
            ("agent", false),
        ] {
            let p = project(
                &format!("hr-{scenario}-{reporter}"),
                &[("a.test.ts", file)],
                esbuild,
            );
            let root = p.dir.display().to_string();
            let (argv, env): (&[&str], _) = if reporter == "agent" {
                (
                    &[],
                    vec![("TURBO_LEGACY_REPORTER", "0"), ("AI_AGENT", "claude-code")],
                )
            } else {
                (
                    &["--reporter", reporter],
                    vec![("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
                )
            };
            let r = turbo_test(&p.dir, argv, &env);
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
