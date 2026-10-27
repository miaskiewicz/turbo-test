//! Module-runner (esbuild project) semantics that follow vitest/Node: a module's own load error
//! is the reported cause (entry import, `require()`, dynamic `import()`); `import.meta`
//! (url/filename/dirname) and CSS-module class-name proxies; a mock factory that throws fails the
//! import of the mocked module with that error; a setup file's `vi.mock` is hoisted above its own
//! imports; reading an export a `vi.mock` factory did not return throws.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn esbuild_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
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

/// The `ERROR <file>  (<msg>)` load-error line for `file`.
fn load_error_line<'a>(r: &'a common::Run, file: &str) -> &'a str {
    let suffix = format!("{file}  (");
    r.stdout
        .lines()
        .find(|l| l.starts_with("ERROR") && l.contains(&suffix))
        .unwrap_or_else(|| panic!("no load error for {file}\n{}", r.all()))
}

const FRESH_AND_REUSE: [&[(&str, &str)]; 2] = [&[], &[("TURBO_REUSE_ISOLATE", "1")]];

#[test]
fn module_runner_load_errors_keep_their_cause() {
    let p = esbuild_project("rc-load-cause");
    p.file("dep.js", "throw new Error('boom from dep');\n");
    p.file("a.test.js", "import './dep.js';\ntest('x', () => {});\n");
    p.file(
        "e.mjs",
        "export const a = 1;\nthrow new Error('esm body throws');\n",
    );
    p.file(
        "b.test.js",
        r#"test('require of a throwing ES module throws its error', () => {
  expect(() => require('./e.mjs')).toThrow('esm body throws');
});
"#,
    );
    for env in FRESH_AND_REUSE {
        let r = turbo_test(&p.dir, &["a.test.js"], env);
        assert_eq!(r.code, Some(1), "{env:?}\n{}", r.all());
        assert!(
            load_error_line(&r, "a.test.js").ends_with("(Error: boom from dep)"),
            "{env:?}\n{}",
            r.all()
        );
        assert_passes(&turbo_test(&p.dir, &["b.test.js"], env), 1);
    }
}

#[test]
fn esm_graph_dynamic_import_rejects_with_the_module_error() {
    // No esbuild: `import()` reaches the host callback, which loads the CJS module natively.
    let p = Project::new("rc-dyn-cause");
    p.file("package.json", r#"{"name":"rc-dyn-cause"}"#);
    p.file("throws.cjs", "throw new Error('cjs body boom');\n");
    // unreadable source: the load fails without an exception, so the rejection is generic
    std::fs::write(p.path("badutf.mjs"), b"export const x = \"\xff\xfe\";\n").unwrap();
    p.file(
        "a.test.js",
        "test('rejects with the body error', async () => {\n  \
         await expect(import('./throws.cjs')).rejects.toThrow('cjs body boom');\n});\n\
         test('a load without an exception', async () => {\n  \
         await expect(import('./badutf.mjs')).rejects.toThrow('dynamic import failed: ./badutf.mjs');\n});\n",
    );
    assert_passes(&turbo_test(&p.dir, &[], &[]), 2);
}

#[test]
fn module_runner_import_meta_is_the_files_own() {
    let p = esbuild_project("rc-import-meta");
    // the same source in two directories: each module sees its own location
    let meta = "export const url = import.meta.url;\nexport const filename = import.meta.filename;\n\
                export const dirname = import.meta.dirname;\nexport const kind = typeof import.meta;\n";
    p.file("src/meta.ts", meta);
    p.file("src/sub/meta.ts", meta);
    let src = p.path("src/meta.ts");
    let sub = p.path("src/sub/meta.ts");
    let entry = p.path("a.test.ts");
    p.file(
        "a.test.ts",
        &format!(
            r#"import * as m from './src/meta';
import * as s from './src/sub/meta';
test('import.meta of an app module', () => {{
  expect(m.url).toBe('file://{src}');
  expect(m.filename).toBe('{src}');
  expect(m.dirname).toBe('{src_dir}');
  expect(m.kind).toBe('object');
  expect(s.url).toBe('file://{sub}');
  expect(s.dirname).toBe('{sub_dir}');
}});
test('import.meta of the test file', () => {{
  expect(import.meta.url).toBe('file://{entry}');
  expect(import.meta.dirname).toBe('{dir}');
}});
"#,
            src = src.display(),
            src_dir = src.parent().unwrap().display(),
            sub = sub.display(),
            sub_dir = sub.parent().unwrap().display(),
            entry = entry.display(),
            dir = p.dir.display(),
        ),
    );
    for env in [
        &[][..],
        &[("TURBO_NATIVE_CJS", "0")][..],
        &[("TURBO_REUSE_ISOLATE", "1")][..],
    ] {
        assert_passes(&turbo_test(&p.dir, &["a.test.ts"], env), 2);
    }
}

#[test]
fn esm_graph_import_meta_has_filename_and_dirname() {
    // No esbuild: the entry is a native ES module and import.meta comes from the host callback.
    let p = Project::new("rc-import-meta-esm");
    p.file("package.json", r#"{"name":"rc-import-meta-esm"}"#);
    let entry = p.path("a.test.js");
    p.file(
        "a.test.js",
        &format!(
            "test('import.meta', () => {{\n  expect(import.meta.url).toBe('file://{e}');\n  \
             expect(import.meta.filename).toBe('{e}');\n  expect(import.meta.dirname).toBe('{d}');\n}});\n",
            e = entry.display(),
            d = p.dir.display(),
        ),
    );
    assert_passes(&turbo_test(&p.dir, &[], &[]), 1);
}

/// turbo-test#123: `import.meta` (url / filename / dirname, `env`) on every loader path — the
/// module runner's native (oxc) CJS emit and dep bundler, its esbuild transforms and dep bundles,
/// and the ESM graph — in the test file, an app module, a setup file, a `vi.mock` factory (the
/// test file's) and ES-module packages (each module its own location). The report's
/// `Cannot use 'import.meta' outside a module` was an `import.meta` left in CJS output. vitest
/// 4.1.12 on the same project: all 7 pass (its `import.meta.env` reads and writes
/// `process.env`, with `MODE` / `DEV` / `PROD` / `SSR` / `BASE_URL` set there).
#[test]
fn import_meta_works_on_every_loader_path() {
    let files: [(&str, &str); 10] = [
        ("package.json", "{\"name\":\"rc-meta-paths\",\"type\":\"module\"}\n"),
        (
            "vitest.config.mjs",
            "export default { test: { setupFiles: ['./setup.ts'] } }\n",
        ),
        (
            "setup.ts",
            "(globalThis as any).__setupMeta = [import.meta.url, import.meta.env.MODE];\n",
        ),
        (
            "src/app.ts",
            "export const appMeta = () => ({ url: import.meta.url, filename: import.meta.filename, \
             dirname: import.meta.dirname, mode: import.meta.env.MODE, same: import.meta === import.meta });\n",
        ),
        ("src/other.ts", "export const where = (): string => 'real';\n"),
        (
            "node_modules/esm-pkg/package.json",
            "{\"name\":\"esm-pkg\",\"type\":\"module\",\"exports\":\"./lib/index.js\"}\n",
        ),
        (
            "node_modules/esm-pkg/lib/index.js",
            "import { h } from './sub/h.js';\nexport const own = () => import.meta.url;\nexport const sub = h;\n",
        ),
        (
            "node_modules/esm-pkg/lib/sub/h.js",
            "export const h = () => [import.meta.url, import.meta.dirname];\n",
        ),
        (
            "node_modules/mjsdep/package.json",
            "{\"name\":\"mjsdep\",\"main\":\"index.mjs\"}\n",
        ),
        (
            "node_modules/mjsdep/index.mjs",
            "export const asset = () => new URL('./x.txt', import.meta.url).href;\n",
        ),
    ];
    let test = r#"import { expect, it, vi } from 'vitest';
import { appMeta } from './src/app';
import { where } from './src/other';
import { own, sub } from 'esm-pkg';
import { asset } from 'mjsdep';
vi.mock('./src/other', () => ({ where: (): string => import.meta.url }));
const P = process.cwd();
it('the test file', () => {
  expect([import.meta.url, import.meta.filename, import.meta.dirname]).toEqual([`file://${P}/a.test.ts`, `${P}/a.test.ts`, P]);
  expect(import.meta).toBe(import.meta);
});
it('an app module', () => {
  expect(appMeta()).toEqual({ url: `file://${P}/src/app.ts`, filename: `${P}/src/app.ts`, dirname: `${P}/src`, mode: 'test', same: true });
});
it('ES-module packages', () => {
  expect(own()).toBe(`file://${P}/node_modules/esm-pkg/lib/index.js`);
  expect(sub()).toEqual([`file://${P}/node_modules/esm-pkg/lib/sub/h.js`, `${P}/node_modules/esm-pkg/lib/sub`]);
  expect(asset()).toBe(`file://${P}/node_modules/mjsdep/x.txt`);
});
it('a setup file', () => { expect((globalThis as any).__setupMeta).toEqual([`file://${P}/setup.ts`, 'test']); });
it('a mock factory is the test file', () => { expect(where()).toBe(`file://${P}/a.test.ts`); });
it('import.meta.env', () => {
  const env = import.meta.env as any;
  expect([env.MODE, env.DEV, env.PROD, env.SSR, env.BASE_URL, env.NODE_ENV, env.TT_UNSET]).toEqual(['test', true, false, true, '/', 'test', undefined]);
  expect([process.env.MODE, process.env.DEV, process.env.PROD, process.env.SSR, process.env.BASE_URL]).toEqual(['test', '1', '', '1', '/']);
  process.env.TT_META_X = 'a';
  expect(env.TT_META_X).toBe('a');
  env.TT_META_Y = 'b';
  expect(process.env.TT_META_Y).toBe('b');
  env.DEV = false;
  expect([process.env.DEV, env.DEV]).toEqual(['', false]);
  env.DEV = true;
  expect(['MODE' in env, 'TT_UNSET' in env]).toEqual([true, false]);
  expect(import.meta.env).toBe(import.meta.env);
});
"#;
    for esbuild in [true, false] {
        let p = Project::new(if esbuild {
            "rc-meta-paths"
        } else {
            "rc-meta-paths-esm"
        });
        if esbuild {
            p.with_esbuild();
        }
        for (f, c) in files {
            p.file(f, c);
        }
        p.file("a.test.ts", test);
        // vitest's web transform mode: SSR is off under a DOM environment
        p.file(
            "b.test.ts",
            "// @vitest-environment happy-dom\nimport { expect, it } from 'vitest';\n\
             it('SSR under a DOM environment', () => {\n\
             \x20 expect([(import.meta.env as any).SSR, process.env.SSR, (import.meta.env as any).DEV]).toEqual([false, '', true]);\n\
             });\n",
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[
                &[],
                &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")],
                &[("TURBO_REUSE_ISOLATE", "1")],
            ]
        } else {
            &[&[], &[("TURBO_REUSE_ISOLATE", "1")]]
        };
        for env in envs {
            let r = turbo_test(&p.dir, &[], env);
            assert!(
                r.stdout.contains("Tests  7 passed (7)"),
                "esbuild={esbuild} {env:?}\n{}",
                r.all()
            );
            assert_eq!(r.code, Some(0), "esbuild={esbuild} {env:?}\n{}", r.all());
        }
    }
}

/// turbo-test#130: vitest 4.1.12 gives every module `import.meta.resolve` (Node's, from the
/// module), `import.meta.main` (`false`), `import.meta.glob` (statically replaced at a full-name
/// call: eager / lazy, `import`, negated and array patterns, `exhaustive`, relative and
/// root-absolute patterns, the module itself skipped; anything else throws) and, in the test file,
/// the non-enumerable `import.meta.vitest`. turbo-test gave `url, filename, dirname, env` only.
/// vitest 4.1.12 on the same project: all 6 pass.
#[test]
fn import_meta_resolve_main_and_glob_on_every_loader_path() {
    let files: [(&str, &str); 10] = [
        ("package.json", "{\"name\":\"rc-meta-members\",\"type\":\"module\"}\n"),
        (
            "node_modules/esm-pkg/package.json",
            "{\"name\":\"esm-pkg\",\"type\":\"module\",\"exports\":\"./lib/index.js\"}\n",
        ),
        ("node_modules/esm-pkg/lib/index.js", "export const own = 1;\n"),
        ("src/mods/a.ts", "export default 'A'; export const n = 1;\n"),
        ("src/mods/b.js", "export default 'B'; export const n = 2;\n"),
        ("src/mods/deep/z.ts", "export default 'Z'; export const n = 26;\n"),
        ("src/mods/.hidden.ts", "export const n = 0;\n"),
        ("src/mods/node_modules/x/i.ts", "export const n = 9;\n"),
        (
            "src/mods/user.ts",
            "export const siblings = (): string[] => Object.keys(import.meta.glob('./*.ts', { eager: true }));\n\
             export const up = (): string[] => Object.keys(import.meta.glob('../*.ts'));\n\
             export const resolveHere = (s: string): string => import.meta.resolve(s);\n\
             export const names = (): string[] => Object.getOwnPropertyNames(import.meta);\n",
        ),
        ("src/app.ts", "export const app = 'app';\n"),
    ];
    let test = r#"import { expect, it, vi } from 'vitest';
import { siblings, up, resolveHere, names } from './src/mods/user';
const P = process.cwd();
it('import.meta has vitest members', () => {
  expect(Object.getOwnPropertyNames(import.meta)).toEqual(['filename', 'dirname', 'url', 'env', 'resolve', 'glob', 'main', 'vitest']);
  expect(Object.keys(import.meta)).toEqual(['filename', 'dirname', 'url', 'env', 'resolve', 'glob', 'main']);
  expect(names()).toEqual(['filename', 'dirname', 'url', 'env', 'resolve', 'glob', 'main']);
  expect(import.meta.main).toBe(false);
  expect([import.meta.resolve.name, import.meta.resolve.length, import.meta.glob.name, import.meta.glob.length]).toEqual(['resolve', 2, 'glob', 0]);
  const v = (import.meta as any).vitest;
  expect([v.expect === expect, v.vi === vi, v.it === it]).toEqual([true, true, true]);
  expect(Object.getOwnPropertyDescriptor(import.meta, 'vitest')!.enumerable).toBe(false);
});
it('import.meta.resolve', () => {
  expect(import.meta.resolve('./src/app.ts')).toBe(`file://${P}/src/app.ts`);
  expect(import.meta.resolve('./src/app')).toBe(`file://${P}/src/app`);
  expect(import.meta.resolve('./no such.js')).toBe(`file://${P}/no%20such.js`);
  expect(import.meta.resolve('esm-pkg')).toBe(`file://${P}/node_modules/esm-pkg/lib/index.js`);
  expect([import.meta.resolve('fs'), import.meta.resolve('node:path')]).toEqual(['node:fs', 'node:path']);
  expect(import.meta.resolve('./x', 'file:///tmp/a/')).toBe('file:///tmp/a/x');
  expect(import.meta.resolve('data:text/javascript,1')).toBe('data:text/javascript,1');
  expect(resolveHere('./a.ts')).toBe(`file://${P}/src/mods/a.ts`);
  expect(resolveHere('../app.ts')).toBe(`file://${P}/src/app.ts`);
  let err: any;
  try { import.meta.resolve('nopkg-xyz'); } catch (e) { err = e; }
  expect([err?.name, err?.code, err?.message]).toEqual(['Error', 'ERR_MODULE_NOT_FOUND', `Cannot find package 'nopkg-xyz' imported from ${P}/a.test.ts`]);
});
it('import.meta.glob not called by its full name throws', () => {
  const g = import.meta.glob;
  expect(() => g('./src/*.ts')).toThrow(new Error('[module runner] "import.meta.glob" is statically replaced during file transformation. Make sure to reference it by the full name.'));
});
it('import.meta.glob eager', async () => {
  const mods = import.meta.glob('./src/mods/**/*', { eager: true });
  expect(Object.keys(mods)).toEqual(['./src/mods/a.ts', './src/mods/b.js', './src/mods/deep/z.ts', './src/mods/user.ts']);
  expect(mods['./src/mods/a.ts']).toBe(await import('./src/mods/a.ts'));
  expect(import.meta.glob(['./src/mods/*.ts', './src/mods/*.js', '!./src/mods/a.ts'], { eager: true, import: 'n' })).toEqual({ './src/mods/b.js': 2 });
  expect(import.meta.glob('./src/mods/*.{ts,js}', { eager: true, import: 'default' })).toEqual({ './src/mods/a.ts': 'A', './src/mods/b.js': 'B', './src/mods/user.ts': undefined });
  expect(Object.keys(import.meta.glob('./src/mods/**/*.ts', { eager: true, exhaustive: true }))).toEqual(['./src/mods/.hidden.ts', './src/mods/a.ts', './src/mods/deep/z.ts', './src/mods/node_modules/x/i.ts', './src/mods/user.ts']);
  expect(Object.keys(import.meta.glob('/src/*.ts', { eager: true }))).toEqual(['/src/app.ts']);
  expect(import.meta.glob('./nomatch/*.ts')).toEqual({});
});
it('import.meta.glob lazy', async () => {
  const lazy = import.meta.glob('./src/mods/*.ts');
  expect(Object.keys(lazy)).toEqual(['./src/mods/a.ts', './src/mods/user.ts']);
  expect(await lazy['./src/mods/a.ts']()).toBe(await import('./src/mods/a.ts'));
  const named = import.meta.glob<string>('./src/mods/deep/*.ts', { import: 'default' });
  expect(await named['./src/mods/deep/z.ts']()).toBe('Z');
});
it('import.meta.glob in an app module is relative to it and skips the module itself', () => {
  expect(siblings()).toEqual(['./a.ts']);
  expect(up()).toEqual(['../app.ts']);
});
"#;
    for esbuild in [true, false] {
        let p = Project::new(if esbuild {
            "rc-meta-members"
        } else {
            "rc-meta-members-esm"
        });
        if esbuild {
            p.with_esbuild();
        }
        for (f, c) in files {
            p.file(f, c);
        }
        p.file("a.test.ts", test);
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[
                &[],
                &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")],
                &[("TURBO_REUSE_ISOLATE", "1")],
            ]
        } else {
            &[&[], &[("TURBO_REUSE_ISOLATE", "1")]]
        };
        for env in envs {
            let r = turbo_test(&p.dir, &[], env);
            assert!(
                r.stdout.contains("Tests  6 passed (6)"),
                "esbuild={esbuild} {env:?}\n{}",
                r.all()
            );
            assert_eq!(r.code, Some(0), "esbuild={esbuild} {env:?}\n{}", r.all());
        }
    }
}

/// turbo-test#131: `vi.spyOn` on an app module's namespace export reaches the modules importing
/// it, on the ESM-graph loader (no esbuild) as on the module runner: `export const` / `function` /
/// `class`, a renamed local (`export { a as b }`), `export default` (expression and anonymous
/// function), re-exports (`export { x as y } from`, `export *`), with `mockRestore` putting the
/// original back. The ESM graph's namespaces are V8's (not writable), so the spy was silently
/// dropped: `expected 'hi a!' to be 'spied!'`. vitest 4.1.12 on the same project: all 4 pass.
#[test]
#[ignore = "turbo-test#131: fails until the fix"]
fn spy_on_a_namespace_export_reaches_its_importers_on_every_loader_path() {
    let files: [(&str, &str); 6] = [
        (
            "package.json",
            "{\"name\":\"rc-ns-spy\",\"type\":\"module\"}\n",
        ),
        (
            "src/dep.ts",
            r#"export const greet = (n: string): string => 'hi ' + n;
export function fn(): string { return 'fn'; }
export class K { static who(): string { return 'K'; } }
const inner = (): string => 'inner';
export { inner as renamed };
export let counter = 1;
export default (): string => 'default';
"#,
        ),
        (
            "src/deffn.ts",
            r#"export default function (): string { return 'deffn'; }
"#,
        ),
        (
            "src/barrel.ts",
            r#"export { greet as hello } from './dep';
export * from './star';
"#,
        ),
        (
            "src/star.ts",
            r#"export const starred = (): string => 'star';
"#,
        ),
        (
            "src/svc.ts",
            r#"import def, { greet, fn, renamed } from './dep';
import deffn from './deffn';
import { hello, starred } from './barrel';
export const all = (): string[] => [greet('a'), fn(), renamed(), def(), deffn(), hello('b'), starred()];
"#,
        ),
    ];
    let test = r#"import { expect, it, vi, afterEach } from 'vitest';
import * as dep from './src/dep';
import * as deffn from './src/deffn';
import * as barrel from './src/barrel';
import * as star from './src/star';
import { greet } from './src/dep';
import { all } from './src/svc';
afterEach(() => { vi.restoreAllMocks(); });
it('spies on every export form reach the importer', () => {
  vi.spyOn(dep, 'greet').mockReturnValue('G');
  vi.spyOn(dep, 'fn').mockReturnValue('F');
  vi.spyOn(dep, 'renamed').mockReturnValue('R');
  vi.spyOn(dep, 'default').mockReturnValue('D');
  vi.spyOn(deffn, 'default').mockReturnValue('DF');
  vi.spyOn(star, 'starred').mockReturnValue('S');
  expect(all()).toEqual(['G', 'F', 'R', 'D', 'DF', 'G', 'S']);
  expect(greet('x')).toBe('G');
  expect(dep.greet).toBe(greet);
  expect(vi.isMockFunction(dep.greet)).toBe(true);
});
it('restore puts the originals back', () => {
  expect(all()).toEqual(['hi a', 'fn', 'inner', 'default', 'deffn', 'hi b', 'star']);
  expect(vi.isMockFunction(dep.greet)).toBe(false);
});
it('spy through a re-exporting barrel', () => {
  vi.spyOn(barrel, 'hello').mockReturnValue('H');
  vi.spyOn(barrel, 'starred').mockReturnValue('BS');
  expect([barrel.hello('z'), barrel.starred()]).toEqual(['H', 'BS']);
  expect(all()).toEqual(['hi a', 'fn', 'inner', 'default', 'deffn', 'H', 'BS']);
});
it('static method and namespace tag', () => {
  vi.spyOn(dep.K, 'who').mockReturnValue('k');
  expect(dep.K.who()).toBe('k');
  expect(Object.prototype.toString.call(dep)).toBe('[object Module]');
});
"#;
    for esbuild in [false, true] {
        let p = Project::new(if esbuild {
            "rc-ns-spy"
        } else {
            "rc-ns-spy-esm"
        });
        if esbuild {
            p.with_esbuild();
        }
        for (f, c) in files {
            p.file(f, c);
        }
        p.file("spy.test.ts", test);
        for env in [&[][..], &[("TURBO_REUSE_ISOLATE", "1")][..]] {
            let r = turbo_test(&p.dir, &[], env);
            assert!(
                r.stdout.contains("Tests  4 passed (4)"),
                "esbuild={esbuild} {env:?}\n{}",
                r.all()
            );
            assert_eq!(r.code, Some(0), "esbuild={esbuild} {env:?}\n{}", r.all());
        }
    }
}

#[test]
fn import_meta_keeps_coverage_line_mapping() {
    let p = esbuild_project("rc-import-meta-cov");
    p.file(
        "src/meta.ts",
        "export const url = import.meta.url;\nexport function hit() {\n  return 'hit';\n}\n\
         export function miss() {\n  return 'miss';\n}\n",
    );
    p.file(
        "a.test.ts",
        "import { url, hit } from './src/meta';\n\
         test('t', () => { expect(url.startsWith('file://')).toBe(true); expect(hit()).toBe('hit'); });\n",
    );
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage",
            "--coverage-reporter",
            "lcov",
            "--coverage-dir",
            "cov",
        ],
        &[],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    let lcov = std::fs::read_to_string(p.path("cov/lcov.info")).unwrap();
    for da in ["DA:1,1", "DA:3,1", "DA:6,0"] {
        assert!(lcov.lines().any(|l| l == da), "{da}\n{lcov}");
    }
}

#[test]
fn module_runner_css_modules_are_class_name_proxies() {
    let p = esbuild_project("rc-css-modules");
    p.file("s.module.css", ".foo { color: red }\n");
    p.file("plain.css", "body { margin: 0 }\n");
    p.file(
        "a.test.ts",
        r#"import s from './s.module.css';
import './plain.css';
test('css module default import maps class names to themselves', () => {
  expect(s.foo).toBe('foo');
  expect(s['btn-primary']).toBe('btn-primary');
  expect(`${s.a} ${s.b}`).toBe('a b');
});
test('plain css is a side-effect import', () => {
  expect(require('./plain.css')).toBeDefined();
});
"#,
    );
    for env in [
        &[][..],
        &[("TURBO_NATIVE_CJS", "0")][..],
        &[("TURBO_REUSE_ISOLATE", "1")][..],
    ] {
        assert_passes(&turbo_test(&p.dir, &["a.test.ts"], env), 2);
    }
}

/// vitest's wrapper for a factory error; the cause follows it.
const MOCK_ERR: &str = "[vitest] There was an error when mocking a module.";

#[test]
fn module_runner_mock_factory_errors_fail_the_import() {
    let p = esbuild_project("rc-factory-error");
    p.file("m.js", "export const v = 'real';\n");
    p.file("n.js", "export const n = 'real-n';\n");
    p.file(
        "hoisted.test.js",
        "import { v } from './m.js';\n\
         vi.mock('./m.js', () => { throw new Error('factory boom') });\n\
         test('t', () => { expect(v).toBe('never'); });\n",
    );
    p.file(
        "async.test.js",
        "import { n } from './n.js';\n\
         vi.mock('./n.js', async () => { throw new Error('async boom') });\n\
         test('t', () => { expect(n).toBe('never'); });\n",
    );
    // jest semantics: the factory's own error propagates from the require
    p.file(
        "jest.test.js",
        "import { v } from './m.js';\n\
         jest.mock('./m.js', () => { throw new Error('jest boom') });\n\
         test('t', () => { expect(v).toBe('never'); });\n",
    );
    p.file(
        "domock.test.js",
        r#"import { n } from './n.js';
// the prepass runs factories outside the module scope (`local` is not defined there); the
// module's own call, which sees it, is the one that counts
const local = 'from-module-scope';
vi.mock('./m.js', () => ({ v: local }));
// a factory is only reached by importing its module: an unimported one never fails the file
vi.mock('./unused.js', () => { throw new Error('never imported'); });
test('module-scope factory', () => { expect(require('./m.js').v).toBe('from-module-scope'); });
test('doMock factory error surfaces on the next import', () => {
  expect(n).toBe('real-n');
  vi.doMock('./n.js', () => { throw new Error('do boom'); });
  expect(() => require('./n.js')).toThrow('Caused by: Error: do boom');
  vi.doMock('./n.js', () => ({ n: 'fixed' }));
  expect(require('./n.js').n).toBe('fixed');
});
"#,
    );
    for env in FRESH_AND_REUSE {
        for (file, cause) in [
            ("hoisted.test.js", "Error: factory boom"),
            ("async.test.js", "Error: async boom"),
        ] {
            let r = turbo_test(&p.dir, &[file], env);
            assert_eq!(r.code, Some(1), "{env:?}\n{}", r.all());
            let line = load_error_line(&r, file);
            assert!(
                line.contains(MOCK_ERR) && line.ends_with(&format!(" Caused by: {cause})")),
                "{env:?}\n{}",
                r.all()
            );
        }
        let r = turbo_test(&p.dir, &["jest.test.js"], env);
        assert!(
            load_error_line(&r, "jest.test.js").ends_with("(Error: jest boom)"),
            "{env:?}\n{}",
            r.all()
        );
        assert_passes(&turbo_test(&p.dir, &["domock.test.js"], env), 2);
    }
}

#[test]
fn esm_graph_mock_factory_error_fails_the_import() {
    // No esbuild: the entry's factories are run natively before it links.
    let p = Project::new("rc-factory-error-esm");
    p.file("package.json", r#"{"name":"rc-factory-error-esm"}"#);
    p.file("m.js", "export const v = 'real';\n");
    p.file(
        "a.test.js",
        "import { v } from './m.js';\n\
         vi.mock('./m.js', () => { throw new Error('factory boom') });\n\
         test('t', () => { expect(v).toBe('never'); });\n",
    );
    p.file(
        "dyn.test.js",
        "vi.mock('./m.js', () => { throw new Error('dyn boom') });\n\
         test('rejects', async () => {\n  \
         await expect(import('./m.js')).rejects.toThrow('Caused by: Error: dyn boom');\n});\n",
    );
    let r = turbo_test(&p.dir, &["a.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    let line = load_error_line(&r, "a.test.js");
    assert!(
        line.contains(MOCK_ERR) && line.ends_with(" Caused by: Error: factory boom)"),
        "{}",
        r.all()
    );
    assert_passes(&turbo_test(&p.dir, &["dyn.test.js"], &[]), 1);
}

#[test]
fn setup_file_mocks_are_hoisted_above_its_own_imports() {
    let p = esbuild_project("rc-setup-hoist");
    p.file(
        "vitest.config.js",
        "export default { test: { setupFiles: ['./setup.js'] } };\n",
    );
    p.file(
        "m.js",
        "export const v = 'real';\nexport const f = () => 'real-f';\n",
    );
    p.file(
        "setup.js",
        r#"import { v, f } from './m.js';
const { spy } = vi.hoisted(() => ({ spy: vi.fn(() => 'mocked-f') }));
vi.mock('./m.js', () => ({ v: 'mocked', f: spy }));
globalThis.SETUP_SAW = v;
globalThis.SETUP_F = f;
"#,
    );
    p.file(
        "a.test.js",
        r#"import { v, f } from './m.js';
const own = vi.hoisted(() => 'entry-hoisted');
test('the setup file saw its own mock', () => {
  expect(globalThis.SETUP_SAW).toBe('mocked');
});
test('setup and test file share one mock', () => {
  expect(v).toBe('mocked');
  expect(f).toBe(globalThis.SETUP_F);
  expect(f()).toBe('mocked-f');
});
test("the setup's vi.hoisted values do not leak into the test file's", () => {
  expect(own).toBe('entry-hoisted');
});
"#,
    );
    for env in FRESH_AND_REUSE {
        assert_passes(&turbo_test(&p.dir, &[], env), 3);
    }
}

/// vitest's error for reading a name the factory did not return.
fn no_export(name: &str, spec: &str) -> String {
    format!(
        "[vitest] No \"{name}\" export is defined on the \"{spec}\" mock. \
         Did you want to return the actual module?"
    )
}

#[test]
fn module_runner_missing_mock_exports_throw_on_access() {
    let p = esbuild_project("rc-strict-mock");
    p.file(
        "vitest.config.js",
        "export default { test: { setupFiles: ['./setup.js'] } };\n",
    );
    p.file(
        "m.js",
        "export const v = 'real';\nexport const w = 'real-w';\n",
    );
    p.file(
        "n.js",
        "export const a = 'real-a';\nexport const b = 'real-b';\n",
    );
    p.file(
        "j.js",
        "export const a = 'real-a';\nexport const b = 'real-b';\n",
    );
    p.file(
        "o.js",
        "export const o = 'real-o';\nexport const p = 'real-p';\n",
    );
    p.file(
        "setup.js",
        "import { v } from './m.js';\nvi.mock('./m.js', () => ({ v: 'mocked' }));\n\
         globalThis.SETUP_SAW = v;\n",
    );
    p.file(
        "a.test.js",
        &format!(
            r#"import {{ w }} from './m.js';
import * as n from './n.js';
import {{ b as jb }} from './j.js';
import {{ p }} from './o.js';
vi.mock('./n.js', () => ({{ a: 'mock-a' }}));
jest.mock('./j.js', () => ({{ a: 'mock-a' }}));
vi.mock('./o.js', async () => ({{ o: 'mock-o' }}));
test('a setup mock: present names read, missing ones throw', () => {{
  expect(globalThis.SETUP_SAW).toBe('mocked');
  expect(() => w).toThrow('{w}');
}});
test('a test-file mock', () => {{
  expect(n.a).toBe('mock-a');
  expect(() => n.b).toThrow('{b}');
  // interop probes, symbols and inherited members never throw
  expect(n.then).toBe(undefined);
  expect(n.__esModule).toBe(undefined);
  expect(typeof n.hasOwnProperty).toBe('function');
  expect(n[Symbol.toStringTag]).toBe(undefined);
  expect(Object.keys(n)).toEqual(['a']);
}});
test('an async factory', () => {{ expect(() => p).toThrow('{p}'); }});
test('jest.mock keeps jest semantics: a missing name reads undefined', () => {{
  expect(jb).toBe(undefined);
}});
"#,
            w = no_export("w", "./m.js"),
            b = no_export("b", "./n.js"),
            p = no_export("p", "./o.js"),
        ),
    );
    for env in FRESH_AND_REUSE {
        assert_passes(&turbo_test(&p.dir, &[], env), 4);
    }
}

/// A test file whose `vi.mock` factory reads (and assigns) its own top-level bindings: a `let`,
/// a `const` arrow, a function and a class declaration and an import (vitest 4.1.12 passes it).
const SCOPED_FACTORY: &str = r#"import { test, expect, vi } from 'vitest'
import { v, w } from './dep.js'
import { n } from './other.js'
let captured: string = ''
const helper = (): number => 3
function fnDecl(): number { return 4 }
class K { static k = 6 }
test('factory sees the file scope', () => {
  expect(v()).toBe(3 + 4 + 5 + 6)
  w('hi')
  expect(captured).toBe('hi')
})
vi.mock('./dep.js', () => ({
  v: () => helper() + fnDecl() + n + K.k,
  w: (s: string) => { captured = s },
}))
"#;

#[test]
fn a_mock_factory_sees_the_files_top_level_bindings() {
    // turbo-test#72: the hoisted factory ran in the prepass, outside the file's scope: without
    // esbuild every binding was missing, with esbuild all but a shared `let`.
    for esbuild in [false, true] {
        let p = if esbuild {
            esbuild_project("rc-mock-scope-mr")
        } else {
            Project::new("rc-mock-scope-esm")
        };
        p.file(
            "dep.js",
            "export const v = () => 1; export const w = () => {}\n",
        );
        p.file("other.js", "export const n = 5\n");
        p.file("s.test.ts", SCOPED_FACTORY);
        p.file(
            "l.test.js",
            "import { test, expect, vi } from 'vitest'\nimport { v } from './dep.js'\nlet shared = 2\n\
             test('let', () => { expect(v()).toBe(2) })\nvi.mock('./dep.js', () => ({ v: () => shared }))\n",
        );
        let r = turbo_test(&p.dir, &["s.test.ts", "l.test.js"], &[]);
        assert_eq!(r.code, Some(0), "esbuild={esbuild}\n{}", r.all());
        assert!(r.stdout.contains("Tests  2 passed (2)"), "{}", r.all());
    }
}

/// turbo-test#87: `import.meta.url` is the file's percent-encoded file URL (Node 26 / vitest
/// 4.1.12 `pathToFileURL(filename).href`) on both loaders; `filename` / `dirname` are paths.
#[test]
fn import_meta_url_is_percent_encoded() {
    for esbuild in [false, true] {
        let p = Project::new("rc meta dir €");
        if esbuild {
            p.with_esbuild();
            p.file("package.json", r#"{"name":"rc-meta"}"#);
        } else {
            p.file("package.json", r#"{"type":"module"}"#);
        }
        let entry = p.path("a.test.ts").display().to_string();
        let url = format!(
            "file://{}",
            entry.replace(' ', "%20").replace('€', "%E2%82%AC")
        );
        p.file(
            "a.test.ts",
            &format!(
                "import {{ expect, it }} from 'vitest';\n\
                 import {{ fileURLToPath }} from 'node:url';\n\
                 it('import.meta', () => {{\n\
                 \x20 expect([import.meta.url, import.meta.filename, import.meta.dirname]).toEqual([{url:?}, {entry:?}, {dir:?}]);\n\
                 \x20 expect(fileURLToPath(import.meta.url)).toBe(import.meta.filename);\n\
                 }});\n",
                dir = p.dir.display().to_string(),
            ),
        );
        let r = turbo_test(&p.dir, &[], &[]);
        assert_eq!(r.code, Some(0), "esbuild={esbuild}\n{}", r.all());
        assert!(r.stdout.contains("Tests  1 passed (1)"), "{}", r.all());
    }
}
