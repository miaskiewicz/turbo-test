//! Drop-in repros from an external vitest suite (turbo-test#79: 529 React/TS files, happy-dom,
//! `isolate: false`), each a standalone project like the report's: `package.json` with
//! `"type": "module"` and `vitest.config.mjs` = `{ test: { environment: 'happy-dom' } }`, no
//! node_modules (the ESM graph loader). Expected values are vitest 4.1.12's on the same project
//! (Node 26 for `node:` modules, Chrome 156 for the DOM). The V8-abort repro is in
//! runner_b_reuse.rs.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// The report's repro project with `files` (path, contents).
fn dropin(name: &str, files: &[(&str, &str)]) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{\"type\":\"module\"}\n");
    p.file(
        "vitest.config.mjs",
        "export default { test: { environment: 'happy-dom' } }\n",
    );
    for (path, contents) in files {
        p.file(path, contents);
    }
    p
}

/// Assert a clean pass with exactly `n` passing tests.
fn assert_passes(r: &common::Run, n: u32) {
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains(&format!("Tests  {n} passed ({n})")),
        "{}",
        r.all()
    );
}

/// Item 4: SVG elements have `style` (React sets it on SVG icons). The report's repro expects
/// `'1'` back from `flex`; vitest (happy-dom) and Chrome 156 both serialize `1 1 0%`, so the
/// shorthand is compared with an HTML element's (turbo-dom#90 tracks the serialization) and the
/// SVG declaration itself through longhands.
#[test]
fn svg_elements_have_an_inline_style() {
    let p = dropin(
        "dropin-svg-style",
        &[(
            "a.test.ts",
            "import { expect, it } from 'vitest';\n\
             it('svg style', () => {\n\
             \x20 const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');\n\
             \x20 svg.style.flex = '1';\n\
             \x20 const div = document.createElement('div');\n\
             \x20 div.style.flex = '1';\n\
             \x20 expect(svg.style.flex).toBe(div.style.flex);\n\
             \x20 const icon = document.createElementNS('http://www.w3.org/2000/svg', 'path');\n\
             \x20 icon.style.flexGrow = '1';\n\
             \x20 icon.style.fill = 'red';\n\
             \x20 expect([icon.style.flexGrow, icon.style.fill, icon.getAttribute('style')]).toEqual(['1', 'red', 'flex-grow: 1; fill: red;']);\n\
             });\n",
        )],
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}

/// Item 2: `vi.dynamicImportSettled()` exists and waits for in-flight `import()`s (the report
/// calls it in a setup file's `afterAll`), with the real timer even under fake timers.
#[test]
#[ignore = "turbo-test#79: fails until the fix"]
fn vi_dynamic_import_settled_waits_for_pending_imports() {
    let p = dropin(
        "dropin-import-settled",
        &[
            (
                "a.test.ts",
                "import { expect, it, vi } from 'vitest';\n\
                 it('exists', () => { expect(typeof (vi as any).dynamicImportSettled).toBe('function'); });\n",
            ),
            (
                "tla.ts",
                "await new Promise((r) => setTimeout(r, 20));\nexport const x = 1;\n",
            ),
            (
                "b.test.ts",
                "import { expect, it, vi } from 'vitest';\n\
                 it('settles in-flight imports', async () => {\n\
                 \x20 const seen: number[] = [];\n\
                 \x20 import('./tla').then((m) => seen.push(m.x));\n\
                 \x20 expect(seen).toEqual([]);\n\
                 \x20 await vi.dynamicImportSettled();\n\
                 \x20 expect(seen).toEqual([1]);\n\
                 });\n\
                 it('resolves with undefined under fake timers', async () => {\n\
                 \x20 vi.useFakeTimers();\n\
                 \x20 await expect(vi.dynamicImportSettled()).resolves.toBeUndefined();\n\
                 \x20 vi.useRealTimers();\n\
                 });\n",
            ),
        ],
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 3);
}
