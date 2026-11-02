//! The milestone spike binaries (M0 snapshot isolation, M1.1 ESM loader, M1.2/M1.3 CJS interop,
//! M1.4 oxc transform, M5 affected-test detection) run as subprocesses against the repo fixtures
//! they were written for and against throwaway projects, asserting their printed results and
//! their error/usage paths.
#![cfg(unix)]

mod common;
use common::{repo_root, run_exe, Project, Run};

fn m0(cwd: &std::path::Path) -> Run {
    run_exe(env!("CARGO_BIN_EXE_m0-spike"), cwd, &[], &[])
}
fn m1_esm(cwd: &std::path::Path, args: &[&str]) -> Run {
    run_exe(env!("CARGO_BIN_EXE_m1-esm"), cwd, args, &[])
}
fn m1_cjs(cwd: &std::path::Path, args: &[&str]) -> Run {
    run_exe(env!("CARGO_BIN_EXE_m1-cjs"), cwd, args, &[])
}
fn m1_transform(cwd: &std::path::Path, args: &[&str]) -> Run {
    run_exe(env!("CARGO_BIN_EXE_m1-transform"), cwd, args, &[])
}
fn m5(cwd: &std::path::Path, args: &[&str]) -> Run {
    run_exe(env!("CARGO_BIN_EXE_m5-affected"), cwd, args, &[])
}

// ---- M0 ---------------------------------------------------------------------------------------

#[test]
fn m0_reports_every_strategy_and_a_consistent_verdict() {
    let p = Project::new("m0");
    let r = m0(&p.dir);
    let out = &r.stdout;
    assert!(
        out.starts_with("turbo-test M0 — snapshot-isolation spike\ncores: "),
        "{}",
        r.all()
    );
    assert!(
        out.contains("per-mode iters: 300, repeats: 5 (median reported)"),
        "{}",
        r.all()
    );
    assert!(
        out.contains("isolate creation (once per worker): "),
        "{}",
        r.all()
    );
    assert!(
        out.contains("   funcs   blob(KB)      rebuild  reuse+reset     snapshot       shared"),
        "{}",
        r.all()
    );
    // one numeric row per framework weight
    for w in ["0", "500", "2000", "8000"] {
        let row = out
            .lines()
            .find(|l| l.split_whitespace().next() == Some(w))
            .unwrap_or_else(|| panic!("no row for weight {w}:\n{out}"));
        let cols: Vec<f64> = row.split_whitespace().map(|c| c.parse().unwrap()).collect();
        assert_eq!(cols.len(), 6, "{row}");
        assert!(
            cols[1..].iter().all(|v| *v > 0.0),
            "positive blob size + timings: {row}"
        );
    }
    assert!(
        out.contains(" isolates): ") && out.contains(" contexts/sec"),
        "{out}"
    );
    assert!(
        out.contains("--- vs STOCK VITEST per-file environment setup (real corpus) ---"),
        "{out}"
    );
    assert!(
        out.contains("=> Keep FULL per-file isolation (fresh context)."),
        "{out}"
    );

    // the verdict must agree with the printed speedup vs the turbo-dom-shared baseline
    let speedup: f64 = out
        .lines()
        .find(|l| l.contains("stock vitest env, turbo-dom shared"))
        .and_then(|l| l.rsplit("-> ").next())
        .and_then(|x| x.trim_end_matches('x').parse().ok())
        .unwrap_or_else(|| panic!("no speedup line:\n{out}"));
    if speedup >= 5.0 {
        assert_eq!(r.code, Some(0), "{}", r.all());
        assert!(out.contains("==> M0 PASS"), "{out}");
    } else {
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert!(out.contains("==> M0 FAIL"), "{out}");
    }
}

// ---- M1.1 ESM ---------------------------------------------------------------------------------

#[test]
fn m1_esm_default_fixture_passes() {
    let r = m1_esm(&repo_root(), &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("turbo-test M1.1 — native ESM loader"),
        "{}",
        r.all()
    );
    assert!(r.stdout.contains("fixtures/esm/entry.mjs"), "{}", r.all());
    assert!(
        r.stdout.contains("graph compiled: 3 modules"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("instantiated: status=Instantiated"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("[js] entry evaluated, __result=9"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("evaluated: status=Evaluated"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout
            .contains("globalThis.__result = 9  (expected 9)\n==> M1.1 ESM PASS"),
        "{}",
        r.all()
    );
}

#[test]
fn m1_esm_typescript_fixture_passes_through_oxc() {
    let r = m1_esm(&repo_root(), &["fixtures/esm-ts/entry.mts"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("graph compiled: 2 modules"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("[js] ts esm evaluated, __result=9"),
        "{}",
        r.all()
    );
    assert!(r.stdout.contains("==> M1.1 ESM PASS"), "{}", r.all());
}

#[test]
fn m1_esm_resolves_every_candidate_and_cycles() {
    let p = Project::new("m1esm-resolve");
    p.file(
        "entry.mjs",
        "import { x } from './x';\nimport { y } from './y';\nimport { z } from './z';\n\
         import { w } from './w';\nimport { v } from './v';\nimport { d1 } from './d1';\n\
         import { d2 } from './d2';\nimport { d3 } from './d3';\nimport { c } from './cyc.mjs';\n\
         import { abs } from 'ABS';\n\
         log('parts', x, y, z, w, v, d1, d2, d3, c(), abs);\n\
         globalThis.__result = x + y + z + w + v + d1 + d2 + d3 + c() + abs - 1;\n",
    );
    p.file("x.mjs", "export const x = 1;\n");
    p.file("y.js", "export const y = 1;\n");
    p.file("z.mts", "export const z: number = 1;\n");
    p.file("w.ts", "export const w: number = 1;\n");
    p.file("v.tsx", "export const v = (1 as number);\n");
    p.file("d1/index.mjs", "export const d1 = 1;\n");
    p.file("d2/index.js", "export const d2 = 1;\n");
    p.file("d3/index.ts", "export const d3: number = 1;\n");
    p.file(
        "cyc.mjs",
        "import { back } from './cyc2.mjs';\nexport const c = () => back;\n",
    );
    p.file(
        "cyc2.mjs",
        "import { c } from './cyc.mjs';\nexport const back = 1;\nexport const again = () => c;\n",
    );
    let abs = p.file("abs.mjs", "export const abs = 1;\n");
    let entry = p.path("entry.mjs");
    let src = std::fs::read_to_string(&entry)
        .unwrap()
        .replace("ABS", abs.to_str().unwrap());
    std::fs::write(&entry, src).unwrap();

    let r = m1_esm(&p.dir, &["entry.mjs"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("graph compiled: 12 modules"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("[js] parts 1 1 1 1 1 1 1 1 1 1"),
        "{}",
        r.all()
    );
    assert!(r.stdout.contains("==> M1.1 ESM PASS"), "{}", r.all());
}

#[test]
fn m1_esm_wrong_result_fails() {
    let p = Project::new("m1esm-wrong");
    p.file("e.mjs", "globalThis.__result = 8;\n");
    let r = m1_esm(&p.dir, &["e.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout
            .contains("globalThis.__result = 8  (expected 9)\n==> M1.1 ESM FAIL"),
        "{}",
        r.all()
    );
}

#[test]
fn m1_esm_pending_top_level_await_counts_as_evaluated_but_fails_the_check() {
    let p = Project::new("m1esm-tla");
    p.file(
        "e.mjs",
        "await new Promise(() => {});\nglobalThis.__result = 9;\n",
    );
    let r = m1_esm(&p.dir, &["e.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("evaluated: status=Evaluated"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout
            .contains("globalThis.__result = undefined  (expected 9)"),
        "{}",
        r.all()
    );
}

#[test]
fn m1_esm_load_failures() {
    let p = Project::new("m1esm-fail");
    p.file("bare.mjs", "import React from 'react';\n");
    p.file("missing.mjs", "import { a } from './nope.mjs';\n");
    p.file("deep.mjs", "import './missing.mjs';\n");
    p.file("syntax.mjs", "export const = ;\n");
    p.file("imports-syntax.mjs", "import './syntax.mjs';\n");
    p.file("dir/keep.txt", "\n");
    let cases: &[(&str, &str)] = &[
        ("bare.mjs", "bare specifier not yet supported (M4): react"),
        ("missing.mjs", "could not resolve ./nope.mjs from "),
        ("deep.mjs", "could not resolve ./nope.mjs from "),
        ("syntax.mjs", ""),
        ("imports-syntax.mjs", ""),
        ("dir", "read "),
    ];
    for (entry, msg) in cases {
        let r = m1_esm(&p.dir, &[entry]);
        assert_eq!(r.code, Some(1), "{entry}: {}", r.all());
        assert!(r.stderr.contains(msg), "{entry}: {}", r.all());
        assert!(
            r.stderr.contains("graph load failed"),
            "{entry}: {}",
            r.all()
        );
        assert!(!r.stdout.contains("graph compiled"), "{entry}: {}", r.all());
    }
}

#[test]
fn m1_esm_instantiate_and_evaluate_failures() {
    let p = Project::new("m1esm-link");
    p.file("dep.mjs", "export const a = 1;\n");
    p.file("link.mjs", "import { nope } from './dep.mjs';\nnope;\n");
    p.file("throws.mjs", "throw new Error('boom at top level');\n");
    p.file("bad.ts", "export const = ;\n");
    p.file("badts.mjs", "import { x } from './bad.ts';\nx;\n");

    let r = m1_esm(&p.dir, &["link.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("graph compiled: 2 modules"),
        "{}",
        r.all()
    );
    assert!(r.stderr.contains("instantiate failed"), "{}", r.all());

    let r = m1_esm(&p.dir, &["throws.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr.contains("evaluate failed: status=Errored"),
        "{}",
        r.all()
    );

    // a TS transform error is reported and the module compiles empty → its import can't link
    let r = m1_esm(&p.dir, &["badts.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.stderr.contains("bad.ts: parse errors: "), "{}", r.all());
    assert!(r.stderr.contains("instantiate failed"), "{}", r.all());
}

#[test]
fn m1_esm_missing_entry_panics() {
    let p = Project::new("m1esm-noentry");
    let r = m1_esm(&p.dir, &["nope.mjs"]);
    assert_eq!(r.code, Some(101), "{}", r.all());
    assert!(r.stderr.contains("entry file not found"), "{}", r.all());
}

// ---- M1.2/M1.3 CJS + interop ------------------------------------------------------------------

#[test]
fn m1_cjs_default_interop_fixture_passes() {
    let r = m1_cjs(&repo_root(), &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout
            .contains("turbo-test M1.2/M1.3 — CJS + ESM<->CJS interop"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains("fixtures/interop/entry.mjs"),
        "{}",
        r.all()
    );
    assert!(r.stdout.contains("loaded: 1 esm, 2 cjs"), "{}", r.all());
    assert!(
        r.stdout.contains("[js] hi esm|1.0.0|hi cjs v1.0.0"),
        "{}",
        r.all()
    );
    assert!(
        r.stdout.contains(
            "globalThis.__out = \"hi esm|1.0.0|hi cjs v1.0.0\"\nexpected         = \"hi esm|1.0.0|hi cjs v1.0.0\"\n==> M1.2/M1.3 INTEROP PASS"
        ),
        "{}",
        r.all()
    );
}

/// A throwaway interop project exercising every resolution candidate, require() error, the
/// require cache, export-shape corner cases and a hostile `__keys` helper.
fn interop_project() -> Project {
    let p = Project::new("m1cjs");
    p.file(
        "entry.mjs",
        "import chaos from './chaos.cjs';\n\
         import k1 from './k1.cjs';\nimport { ok } from './k4.cjs';\n\
         import num from './num.cjs';\nimport dflt, { x } from './dflt.cjs';\n\
         import { e } from './esm';\nimport { e2 } from './esm2.js';\nimport { idx } from './pkg';\n\
         log('k1', k1.k, 'ok', ok, 'num', num, 'dflt', dflt.default, x, 'esm', e, e2, idx);\n\
         globalThis.__out = 'hi esm|1.0.0|hi cjs v1.0.0';\n",
    );
    p.file(
        "chaos.cjs",
        "const tryReq = (s) => { try { return require(s); } catch (err) { log('caught', err.message); } };\n\
         tryReq('./esm.mjs');\ntryReq('./nope');\ntryReq('fs');\ntryReq('./bad-syntax.cjs');\ntryReq('./throws.cjs');\n\
         log('cached', require('./lib') === require('./lib.cjs'));\n\
         const saved = globalThis.__keys;\n\
         globalThis.__keys = 5; tryReq('./k1.cjs');\n\
         globalThis.__keys = () => { throw new Error('keys boom'); }; tryReq('./k2.cjs');\n\
         globalThis.__keys = () => 5; tryReq('./k3.cjs');\n\
         globalThis.__keys = () => { const a = ['ok']; Object.defineProperty(a, 1, { get() { throw new Error('idx boom'); }, enumerable: true }); return a; };\n\
         tryReq('./k4.cjs');\nglobalThis.__keys = saved;\n\
         module.exports = { chaos: true };\n",
    );
    p.file("lib.cjs", "module.exports = { VERSION: '1.0.0' };\n");
    p.file("k1.cjs", "module.exports = { k: 'K1' };\n");
    p.file("k2.cjs", "module.exports = { k: 'K2' };\n");
    p.file("k3.cjs", "module.exports = { k: 'K3' };\n");
    p.file("k4.cjs", "module.exports = { ok: 'OK', other: 1 };\n");
    p.file("num.cjs", "module.exports = 42;\n");
    p.file("dflt.cjs", "module.exports = { default: 'D', x: 'X' };\n");
    p.file("bad-syntax.cjs", "module.exports = ;\n");
    p.file("throws.cjs", "throw new Error('cjs threw');\n");
    p.file("esm.mjs", "export const e = 'E';\n");
    p.file("esm2.js", "export { e as e2 } from './esm.mjs';\n");
    p.file("pkg/index.js", "export const idx = 'IDX';\n");
    p
}

#[test]
fn m1_cjs_interop_corner_cases() {
    let p = interop_project();
    let r = m1_cjs(&p.dir, &["entry.mjs"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let out = &r.stdout;
    for line in [
        "[js] caught require() of ESM not supported (M1): ./esm.mjs",
        "[js] caught cannot resolve ./nope",
        "[js] caught cannot resolve fs",
        "[js] caught failed to load ./bad-syntax.cjs",
        "[js] caught failed to load ./throws.cjs",
        "[js] cached true",
        "[js] caught keys boom",
        "[js] caught idx boom",
        // k1's names were lost (non-function __keys) but its default still works; k4 kept `ok`
        "[js] k1 K1 ok OK num 42 dflt D X esm E E IDX",
        "==> M1.2/M1.3 INTEROP PASS",
    ] {
        assert!(out.contains(line), "missing {line:?}\n{}", r.all());
    }
    assert!(
        r.stderr.contains("could not resolve ./nope from "),
        "{}",
        r.all()
    );
    assert!(
        r.stderr
            .contains("bare specifier not yet supported (M4): fs"),
        "{}",
        r.all()
    );
    // entry, esm.mjs, esm2.js, pkg/index.js are ESM; the 8 .cjs files that loaded are CJS
    // (bad-syntax.cjs and throws.cjs never register)
    assert!(out.contains("loaded: 4 esm, 8 cjs"), "{}", r.all());
}

#[test]
fn m1_cjs_named_import_lost_with_keys_is_a_link_error() {
    let p = interop_project();
    // k1 was registered with NO named exports (hostile __keys) → `{ k }` cannot link
    p.file(
        "entry.mjs",
        "import './chaos.cjs';\nimport { k } from './k1.cjs';\nk;\n",
    );
    let r = m1_cjs(&p.dir, &["entry.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.stderr.contains("instantiate failed"), "{}", r.all());
}

#[test]
fn m1_cjs_failures() {
    let p = Project::new("m1cjs-fail");
    p.file(
        "lib.cjs",
        "module.exports = { a: 1, get boom() { throw new Error('getter'); } };\n",
    );
    p.file("bare.mjs", "import x from 'react';\n");
    p.file("missing.mjs", "import x from './nope.cjs';\n");
    p.file("deep.mjs", "import './missing.mjs';\n");
    p.file("cjs-throws.mjs", "import x from './throws.cjs';\n");
    p.file("throws.cjs", "throw new Error('no');\n");
    p.file("syntax.mjs", "export const = ;\n");
    p.file("link.mjs", "import { nope } from './lib.cjs';\n");
    p.file("eval.mjs", "throw new Error('top');\n");
    p.file(
        "getter.mjs",
        "import { a } from './lib.cjs';\nglobalThis.__out = String(a);\n",
    );
    p.file("wrong.mjs", "globalThis.__out = 'nope';\n");
    p.file("dir/keep.txt", "\n");

    for (entry, err) in [
        ("bare.mjs", "bare specifier not yet supported (M4): react"),
        ("missing.mjs", "could not resolve ./nope.cjs from "),
        ("deep.mjs", "could not resolve ./nope.cjs from "),
        ("cjs-throws.mjs", "graph load failed"),
        ("syntax.mjs", "graph load failed"),
        ("dir", "graph load failed"),
    ] {
        let r = m1_cjs(&p.dir, &[entry]);
        assert_eq!(r.code, Some(1), "{entry}: {}", r.all());
        assert!(r.stderr.contains(err), "{entry}: {}", r.all());
        assert!(
            r.stderr.contains("graph load failed"),
            "{entry}: {}",
            r.all()
        );
    }

    let r = m1_cjs(&p.dir, &["link.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.stdout.contains("loaded: 1 esm, 1 cjs"), "{}", r.all());
    assert!(r.stderr.contains("instantiate failed"), "{}", r.all());

    let r = m1_cjs(&p.dir, &["eval.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr.contains("evaluate failed: status=Errored"),
        "{}",
        r.all()
    );

    let r = m1_cjs(&p.dir, &["wrong.mjs"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("globalThis.__out = \"nope\""),
        "{}",
        r.all()
    );
    assert!(r.stdout.contains("\n==> FAIL"), "{}", r.all());

    let r = m1_cjs(&p.dir, &["nope.mjs"]);
    assert_eq!(r.code, Some(101), "{}", r.all());
    assert!(r.stderr.contains("entry not found"), "{}", r.all());
}

#[test]
fn m1_cjs_throwing_export_getter_is_left_unset() {
    let p = Project::new("m1cjs-getter");
    p.file(
        "lib.cjs",
        "module.exports = { a: 1, get boom() { throw new Error('getter'); } };\n",
    );
    p.file(
        "getter.mjs",
        "import { a, boom } from './lib.cjs';\nlog('a', a, typeof boom);\n\
         globalThis.__out = 'hi esm|1.0.0|hi cjs v1.0.0';\n",
    );
    let r = m1_cjs(&p.dir, &["getter.mjs"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("loaded: 1 esm, 1 cjs"), "{}", r.all());
    assert!(r.stdout.contains("[js] a 1 undefined"), "{}", r.all());
}

// ---- M1.4 transform ---------------------------------------------------------------------------

#[test]
fn m1_transform_default_path_passes() {
    // The repo's own fixtures/ts/sample.ts, whose `enum Color` used to trip oxc's
    // `with_enum_eval` debug_assert in this (debug) binary (turbo-test#35 item 1).
    let p = Project::new("m1t");
    p.file(
        "fixtures/ts/sample.ts",
        include_str!("../fixtures/ts/sample.ts"),
    );
    let r = m1_transform(&p.dir, &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.starts_with("turbo-test M1.4 — oxc TS/JSX transform\ninput: fixtures/ts/sample.ts\n--- transformed ---\n"),
        "{}",
        r.all()
    );
    for line in [
        "\tColor[Color[\"Red\"] = 0] = \"Red\";\n",
        "\tColor[Color[\"Green\"] = 1] = \"Green\";\n",
        "\tColor[Color[\"Blue\"] = 2] = \"Blue\";\n",
        "const greet = (p) => `hi ${p.name}, ${Color[Color.Green]}`;\n",
        "export const msg = identity(greet({\n",
    ] {
        assert!(r.stdout.contains(line), "{line}: {}", r.all());
    }
    assert!(
        r.stdout
            .ends_with("--- check ---\n==> M1.4 TRANSFORM PASS\n"),
        "{}",
        r.all()
    );
}

#[test]
fn m1_transform_check_and_error_paths() {
    let p = Project::new("m1t-fail");
    p.file("no-export.ts", "const a: number = 1;\n");
    p.file("leftover.ts", "export const s = 'interface Greeting';\n");
    p.file("bad.ts", "export const = ;\n");
    for f in ["no-export.ts", "leftover.ts"] {
        let r = m1_transform(&p.dir, &[f]);
        assert_eq!(r.code, Some(1), "{f}: {}", r.all());
        assert!(
            r.stdout.ends_with("--- check ---\n==> M1.4 FAIL\n"),
            "{f}: {}",
            r.all()
        );
    }
    let r = m1_transform(&p.dir, &["bad.ts"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr.contains("transform failed: parse errors: "),
        "{}",
        r.all()
    );

    let r = m1_transform(&p.dir, &["nope.ts"]);
    assert_eq!(r.code, Some(101), "{}", r.all());
    assert!(r.stderr.contains("read source"), "{}", r.all());
}

// ---- M5 affected ------------------------------------------------------------------------------

#[test]
fn m5_usage_errors() {
    let p = Project::new("m5-usage");
    for args in [
        &[][..],
        &["--changed"][..],
        &["--changed", "a.js"][..],
        &["t.test.js"][..],
    ] {
        let r = m5(&p.dir, args);
        assert_eq!(r.code, Some(2), "{args:?}: {}", r.all());
        assert_eq!(
            r.stderr, "usage: m5-affected --changed <file>[,<file>] <test files...>\n",
            "{args:?}"
        );
        assert!(r.stdout.is_empty(), "{args:?}: {}", r.all());
    }
}

#[test]
fn m5_prints_the_affected_subset() {
    let p = Project::new("m5");
    p.file(
        "src/a.js",
        "import { b } from './b.js';\nexport const a = b;\n",
    );
    p.file("src/b.js", "export const b = 1;\n");
    p.file("src/c.js", "export const c = 1;\n");
    p.file("a.test.js", "import { a } from './src/a.js';\n");
    p.file("c.test.js", "import { c } from './src/c.js';\n");
    p.file("x.test.js", "test('x', () => {});\n");

    let r = m5(
        &p.dir,
        &[
            "--changed",
            "src/b.js",
            "a.test.js",
            "c.test.js",
            "x.test.js",
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.starts_with("changed: src/b.js\naffected test files (1 of 3):\n  a.test.js\n\nover-selection: 33% of suite selected | analysis "),
        "{}",
        r.all()
    );
    assert!(r.stdout.ends_with(" us\n"), "{}", r.all());

    let r = m5(
        &p.dir,
        &[
            "a.test.js",
            "--changed",
            "src/c.js,x.test.js",
            "c.test.js",
            "x.test.js",
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.starts_with("changed: src/c.js, x.test.js\naffected test files (2 of 3):\n  c.test.js\n  x.test.js\n\nover-selection: 67% "),
        "{}",
        r.all()
    );
}
