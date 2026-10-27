//! Isolate reuse (vitest `isolate: false`, `--no-isolate`, TURBO_REUSE_ISOLATE): one persistent
//! isolate per worker with setup files run once, setup mocks restored between files, a test-file
//! re-mock patched into the shared exports, load errors on the reused path, and the authoritative
//! fresh-isolate retry for failing files. Also the `--coverage` per-file collector.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// Every `SEEN=<n>` value logged by the files of a run, sorted.
fn seen(stdout: &str) -> Vec<u32> {
    let mut v: Vec<u32> = stdout
        .lines()
        .filter_map(|l| l.split("SEEN=").nth(1))
        .map(|n| n.trim().parse().unwrap())
        .collect();
    v.sort();
    v
}

/// Each file appends itself to a context-global list and logs its length: under reuse (one
/// worker) the lengths are 1..=N; with a fresh isolate per file every file logs 1.
const SEEN: &str =
    "globalThis.__seen = (globalThis.__seen || []).concat(1); console.log('SEEN=' + globalThis.__seen.length);\n";

fn reuse_project(name: &str, esbuild: bool) -> Project {
    let p = Project::new(name);
    if esbuild {
        p.with_esbuild();
    }
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p
}

#[test]
fn module_runner_reuse_with_setup_mocks_remocks_dom_and_fresh_retry() {
    let p = reuse_project("rb-reuse-mr", true);
    p.file(
        "vitest.config.ts",
        "export default { test: { isolate: false, setupFiles: ['./setup.ts'] } };\n",
    );
    p.file(
        "src/analytics.ts",
        "export const track = () => 'real-track';\nexport const other = 'o';\n",
    );
    p.file(
        "src/hook.ts",
        "import { track } from './analytics';\nexport const useTrack = () => track();\n",
    );
    p.file(
        "setup.ts",
        "vi.mock('./src/analytics', () => ({ track: () => 'setup-track', other: 'setup-o' }));\n\
         globalThis.__setupRuns = (globalThis.__setupRuns || 0) + 1;\n",
    );
    p.file(
        "a1.test.ts",
        &format!(
            "import {{ useTrack }} from './src/hook';\n{SEEN}\
             test('a1 setup mock', () => {{ expect(useTrack()).toBe('setup-track'); expect(globalThis.__setupRuns).toBe(1); }});\n"
        ),
    );
    p.file(
        "a2.test.ts",
        &format!(
            "import {{ useTrack }} from './src/hook';\nimport {{ other }} from './src/analytics';\n{SEEN}\
             vi.mock('./src/analytics', () => ({{ track: () => 'file-track' }}));\n\
             test('a2 remock', () => {{ expect(useTrack()).toBe('file-track'); expect(globalThis.__setupRuns).toBe(1); }});\n"
        ),
    );
    p.file(
        "a3.test.ts",
        &format!(
            "import {{ useTrack }} from './src/hook';\n{SEEN}\
             test('a3 setup mock again', () => {{ expect(useTrack()).toBe('setup-track'); }});\n"
        ),
    );
    p.file(
        "a4.test.ts",
        &format!("{SEEN}test('a4 fails', () => {{ expect(1).toBe(2); }});\n"),
    );
    p.file(
        "a5.test.tsx",
        &format!(
            "{SEEN}test('a5 dom', () => {{ document.body.innerHTML = '<b>x</b>'; expect(document.body.textContent).toBe('x'); }});\n"
        ),
    );
    p.file(
        "a6.test.tsx",
        &format!(
            "{SEEN}test('a6 dom again', () => {{ document.body.innerHTML = '<i>y</i>'; expect(document.body.textContent).toBe('y'); }});\n"
        ),
    );
    p.file(
        "a7.test.ts",
        "test('a7', () => {});\nthrow new Error('a7 boom');\n",
    );
    let r = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "verbose"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    for ok in [
        "a1 setup mock",
        "a2 remock",
        "a3 setup mock again",
        "a5 dom",
        "a6 dom again",
    ] {
        assert!(r.stdout.contains(&format!("✓ {ok}")), "{ok}\n{}", r.all());
    }
    assert!(
        r.stdout.contains("✗ a4 fails: expected 1 to be 2"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout
            .lines()
            .any(|l| l.starts_with("ERROR") && l.ends_with("a7.test.ts  (Error: a7 boom)")),
        "{}",
        r.all()
    );
    // the passing files shared one context; a4 is re-run on a fresh isolate (logging 1 again)
    let s = seen(&r.stdout);
    assert!(s.iter().any(|&n| n >= 5), "{s:?}\n{}", r.all());
    assert!(s.iter().filter(|&&n| n == 1).count() >= 2, "{s:?}");
}

#[test]
fn esm_graph_reuse_reports_load_errors_and_retries_fresh() {
    let p = reuse_project("rb-reuse-esm", false);
    p.file("lib.mjs", "export const x = 1;\n");
    p.file("b1.test.js", &format!("{SEEN}test('b1', () => {{}});\n"));
    // nested entry: process.cwd() is found by walking up to the package.json
    p.file(
        "deep/er/b2.test.js",
        &format!(
            "{SEEN}test('b2', () => {{ expect(globalThis.__cwd).toBe({:?}); }});\n",
            p.dir.to_string_lossy()
        ),
    );
    p.file(
        "b3.test.js",
        &format!(
            "import {{ x }} from './lib.mjs';\n{SEEN}test('b3', () => {{ expect(x).toBe(1); }});\n"
        ),
    );
    p.file(
        "e-inst.test.js",
        "import { nope } from './lib.mjs'; test('t', () => {});\n",
    );
    p.file("e-graph.test.js", "test('t', () => {}) )\n");
    p.file(
        "e-eval.test.js",
        "test('t', () => {}); throw new Error('reuse boom');\n",
    );
    for env in [&[("TURBO_REUSE_ISOLATE", "1")][..], &[][..]] {
        let args: &[&str] = if env.is_empty() {
            &["--jobs", "1", "--no-isolate"]
        } else {
            &["--jobs", "1"]
        };
        let r = turbo_test(&p.dir, args, env);
        assert_eq!(r.code, Some(1), "{}", r.all());
        let line = |f: &str| {
            r.stdout
                .lines()
                .find(|l| l.contains(&format!(" {f}  ")))
                .unwrap_or_else(|| panic!("{f}\n{}", r.all()))
                .to_string()
        };
        assert!(
            line("e-inst.test.js").contains("(instantiate: SyntaxError: "),
            "{}",
            r.all()
        );
        assert!(
            line("e-graph.test.js").ends_with("(SyntaxError: Unexpected token ')')"),
            "{}",
            r.all()
        );
        assert!(
            line("e-eval.test.js").ends_with("(module errored: Error: reuse boom)"),
            "{}",
            r.all()
        );
        for f in ["b1.test.js", "deep/er/b2.test.js", "b3.test.js"] {
            assert!(line(f).starts_with("PASS"), "{}", r.all());
        }
        assert_eq!(seen(&r.stdout), vec![1, 2, 3], "{}", r.all());
    }
}

#[test]
fn fresh_mode_logs_each_file_in_its_own_context() {
    let p = reuse_project("rb-reuse-off", false);
    p.file("c1.test.js", &format!("{SEEN}test('c1', () => {{}});\n"));
    p.file("c2.test.js", &format!("{SEEN}test('c2', () => {{}});\n"));
    let r = turbo_test(&p.dir, &["--jobs", "1"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(seen(&r.stdout), vec![1, 1]);
}

#[test]
fn coverage_collects_project_sources_only() {
    let p = reuse_project("rb-reuse-cov", true);
    p.file(
        "node_modules/cpkg/index.js",
        "module.exports = { p: () => 1 };\n",
    );
    p.file(
        "node_modules/cpkg/package.json",
        r#"{"name":"cpkg","main":"index.js"}"#,
    );
    p.file(
        "src/math.ts",
        "export function add(a: number, b: number) { return a + b; }\n\
         export function unused(x: number) { if (x > 1) { return 'big'; } return 'small'; }\n",
    );
    p.file("src/data.test.data.ts", "export const td = 1;\n");
    p.file("src/data.spec.data.ts", "export const sd = 2;\n");
    p.file(
        "a.test.ts",
        "import { add } from './src/math';\nimport cpkg from 'cpkg';\nimport { td } from './src/data.test.data';\n\
         import { sd } from './src/data.spec.data';\n\
         test('add', () => { expect(add(1, 2)).toBe(3); expect(cpkg.p()).toBe(1); expect(td + sd).toBe(3); });\n",
    );
    p.file(
        "b.test.ts",
        "import { add } from './src/math';\ntest('add2', () => { expect(add(2, 2)).toBe(4); });\n",
    );
    // reuse is requested but coverage forces the fresh per-file path
    let r = turbo_test(
        &p.dir,
        &["--coverage", "--coverage-reporter", "json-summary"],
        &[("TURBO_REUSE_ISOLATE", "1")],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(p.path("coverage/coverage-summary.json")).unwrap(),
    )
    .unwrap();
    let files: Vec<String> = summary
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| *k != "total")
        .cloned()
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].ends_with("src/math.ts"), "{files:?}");
    let m = &summary[&files[0]];
    assert_eq!(m["functions"]["covered"], 1, "{m}");
    assert!(m["functions"]["total"].as_u64().unwrap() >= 2, "{m}");
}

/// turbo-test#79 (V8 abort): under `isolate: false`, a file whose module throws a SyntaxError
/// while it evaluates leaves the modules after it linked but never evaluated — `y.ts`, bound to
/// the file's mock of `dep.ts`. The next file imports `y.ts` again once that mock is gone; its
/// evaluation must not abort the process (`# Fatal error … Check failed: has_exception()`). vitest
/// 4.1.12: `a.test.ts` fails with the SyntaxError, `b.test.ts` passes against the real `dep.ts`.
#[test]
fn a_file_left_half_linked_does_not_abort_the_next_file() {
    let p = reuse_project("rb-reuse-abort", false);
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "vitest.config.mjs",
        "export default { test: { isolate: false } }\n",
    );
    p.file("dep.ts", "export const v = 0;\n");
    p.file("boom.ts", "JSON.parse('{');\n");
    p.file(
        "y.ts",
        "import { v } from './dep'; export const y = () => v;\n",
    );
    p.file(
        "a.test.ts",
        "import { expect, it, vi } from 'vitest';\n\
         vi.mock('./dep', () => ({ v: 1 }));\n\
         import './boom';\n\
         import { y } from './y';\n\
         it('a', () => { expect(y()).toBe(1); });\n",
    );
    p.file(
        "b.test.ts",
        "import { expect, it } from 'vitest';\n\
         import { y } from './y';\n\
         it('b', () => { expect(y()).toBe(0); });\n",
    );
    let r = turbo_test(&p.dir, &["--jobs", "1"], &[]);
    assert!(!r.all().contains("Fatal error"), "{}", r.all());
    assert_eq!(r.code, Some(1), "{}", r.all());
    let line = |f: &str| {
        r.stdout
            .lines()
            .find(|l| l.contains(&format!(" {f}  ")))
            .unwrap_or_else(|| panic!("{f}\n{}", r.all()))
            .to_string()
    };
    assert!(
        line("a.test.ts").starts_with("ERROR")
            && line("a.test.ts").contains("SyntaxError: Expected property name"),
        "{}",
        r.all()
    );
    assert!(line("b.test.ts").starts_with("PASS"), "{}", r.all());
}
