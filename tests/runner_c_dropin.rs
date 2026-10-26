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

/// Item 3: `node:fs` `readdirSync` lists a directory (the report saw `[]`), with
/// `{ withFileTypes: true }` Dirents and a `URL` argument, in Node's sorted order, and throws
/// Node's ENOENT / ENOTDIR for a missing path / a file; `fs/promises` `readdir` the same.
#[test]
fn readdir_sync_lists_names_dirents_and_url_arguments() {
    let p = dropin(
        "dropin-readdir",
        &[
            ("sub/x.txt", "hi\n"),
            ("sub/b.txt", "b\n"),
            ("sub/inner/.keep", ""),
            ("tree/x.txt", "hi\n"),
            ("tree/b.txt", "b\n"),
            ("tree/inner/.keep", ""),
            (
                "a.test.ts",
                "import { expect, it } from 'vitest';\n\
                 import { readdirSync } from 'node:fs';\n\
                 import { fileURLToPath } from 'node:url';\n\
                 const dir = fileURLToPath(new URL('./sub', import.meta.url)); // sub/x.txt exists\n\
                 it('names', () => { expect(readdirSync(dir)).toEqual(['b.txt', 'inner', 'link', 'x.txt']); });\n\
                 it('withFileTypes', () => { const [e] = readdirSync(dir, { withFileTypes: true }); expect(e?.isFile()).toBe(true); });\n",
            ),
            (
                "b.test.ts",
                "import { expect, it } from 'vitest';\n\
                 import { readdirSync } from 'node:fs';\n\
                 import { readdir } from 'node:fs/promises';\n\
                 import { fileURLToPath } from 'node:url';\n\
                 const url = new URL('./sub', import.meta.url);\n\
                 const dir = fileURLToPath(url);\n\
                 it('a URL or an encoding argument', () => {\n\
                 \x20 expect(readdirSync(url)).toEqual(['b.txt', 'inner', 'link', 'x.txt']);\n\
                 \x20 expect(readdirSync(dir, 'utf8')).toEqual(['b.txt', 'inner', 'link', 'x.txt']);\n\
                 });\n\
                 it('dirents', () => {\n\
                 \x20 const es = readdirSync(url, { withFileTypes: true });\n\
                 \x20 expect(es.map((e) => [e.name, e.isFile(), e.isDirectory(), e.isSymbolicLink(), e.parentPath])).toEqual([\n\
                 \x20   ['b.txt', true, false, false, dir], ['inner', false, true, false, dir],\n\
                 \x20   ['link', false, false, true, dir], ['x.txt', true, false, false, dir],\n\
                 \x20 ]);\n\
                 });\n\
                 it('a missing directory or a file throws', () => {\n\
                 \x20 let err: any, err2: any;\n\
                 \x20 try { readdirSync(dir + '/nope'); } catch (e) { err = e; }\n\
                 \x20 expect([err.message, err.code, err.errno, err.syscall, err.path]).toEqual([\n\
                 \x20   `ENOENT: no such file or directory, scandir '${dir}/nope'`, 'ENOENT', -2, 'scandir', `${dir}/nope`,\n\
                 \x20 ]);\n\
                 \x20 try { readdirSync(dir + '/x.txt'); } catch (e) { err2 = e; }\n\
                 \x20 expect([err2.message, err2.code, err2.errno]).toEqual([`ENOTDIR: not a directory, scandir '${dir}/x.txt'`, 'ENOTDIR', -20]);\n\
                 });\n\
                 it('recursive', () => {\n\
                 \x20 const treeUrl = new URL('./tree', import.meta.url), tree = fileURLToPath(treeUrl);\n\
                 \x20 expect(readdirSync(tree, { recursive: true })).toEqual(['b.txt', 'dlink', 'inner', 'link', 'x.txt', 'dlink/.keep', 'inner/.keep']);\n\
                 \x20 expect(readdirSync(tree + '/', { recursive: true })).toEqual(['b.txt', 'dlink', 'inner', 'link', 'x.txt', 'dlink/.keep', 'inner/.keep']);\n\
                 \x20 expect(readdirSync(treeUrl, { recursive: true, withFileTypes: true }).map((e) => [e.name, e.parentPath.slice(tree.length), e.isDirectory()])).toEqual([\n\
                 \x20   ['b.txt', '', false], ['dlink', '', false], ['inner', '', true], ['link', '', false], ['x.txt', '', false],\n\
                 \x20   ['.keep', '/dlink', false], ['.keep', '/inner', false],\n\
                 \x20 ]);\n\
                 });\n\
                 it('promises.readdir', async () => {\n\
                 \x20 expect(await readdir(url)).toEqual(['b.txt', 'inner', 'link', 'x.txt']);\n\
                 \x20 expect((await readdir(dir, { withFileTypes: true })).map((e) => e.isDirectory())).toEqual([false, true, false, false]);\n\
                 \x20 await expect(readdir(dir + '/nope')).rejects.toMatchObject({ code: 'ENOENT', syscall: 'scandir' });\n\
                 });\n",
            ),
        ],
    );
    std::os::unix::fs::symlink("x.txt", p.path("sub/link")).unwrap();
    std::os::unix::fs::symlink("x.txt", p.path("tree/link")).unwrap();
    std::os::unix::fs::symlink("inner", p.path("tree/dlink")).unwrap();
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 7);
}

/// Item 5 (reduced): a package `exports` subpath whose target is tsc-emitted CommonJS gives its
/// named exports (cjs-module-lexer in Node / vitest). A comment in it mentioning "import " made
/// the loader's module-kind sniff take the file for an ES module.
#[test]
fn a_commonjs_exports_subpath_gives_named_exports() {
    let pkg = "node_modules/@scope/pkg";
    let index = "\"use strict\";\n\
        var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {\n\
        \x20   if (k2 === undefined) k2 = k;\n\
        \x20   var desc = Object.getOwnPropertyDescriptor(m, k);\n\
        \x20   if (!desc || (\"get\" in desc ? !m.__esModule : desc.writable || desc.configurable)) {\n\
        \x20     desc = { enumerable: true, get: function() { return m[k]; } };\n\
        \x20   }\n\
        \x20   Object.defineProperty(o, k2, desc);\n\
        }) : (function(o, m, k, k2) {\n\
        \x20   if (k2 === undefined) k2 = k;\n\
        \x20   o[k2] = m[k];\n\
        }));\n\
        var __exportStar = (this && this.__exportStar) || function(m, exports) {\n\
        \x20   for (var p in m) if (p !== \"default\" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);\n\
        };\n\
        Object.defineProperty(exports, \"__esModule\", { value: true });\n\
        exports.inlineFields = void 0;\n\
        __exportStar(require(\"./blocks\"), exports);\n\
        var heading_1 = require(\"./heading\");\n\
        Object.defineProperty(exports, \"headingFields\", { enumerable: true, get: function () { return heading_1.headingFields; } });\n\
        // Inline fields: import these where the editor renders marks.\n\
        exports.inlineFields = ['b'];\n";
    let p = dropin(
        "dropin-cjs-subpath",
        &[
            (
                &format!("{pkg}/package.json"),
                "{ \"name\": \"@scope/pkg\", \"version\": \"1.0.0\",\n\
                 \x20 \"exports\": { \"./content-fields\": { \"import\": \"./dist/cjs/content-fields/index.js\", \"require\": \"./dist/cjs/content-fields/index.js\" } } }\n",
            ),
            (&format!("{pkg}/dist/cjs/content-fields/index.js"), index),
            (
                &format!("{pkg}/dist/cjs/content-fields/blocks.js"),
                "\"use strict\";\nObject.defineProperty(exports, \"__esModule\", { value: true });\nexports.blockFields = void 0;\nexports.blockFields = ['a'];\n",
            ),
            (
                &format!("{pkg}/dist/cjs/content-fields/heading.js"),
                "\"use strict\";\nObject.defineProperty(exports, \"__esModule\", { value: true });\nexports.headingFields = void 0;\nexports.headingFields = ['h'];\n",
            ),
            (
                "a.test.ts",
                "import { expect, it } from 'vitest';\n\
                 import { blockFields, headingFields, inlineFields } from '@scope/pkg/content-fields';\n\
                 it('named exports', () => { expect([blockFields, headingFields, inlineFields]).toEqual([['a'], ['h'], ['b']]); });\n",
            ),
        ],
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}
