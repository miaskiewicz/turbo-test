//! Mock resolution that follows vitest: a `vi.doMock` applies to a later `import()` on every
//! loader path, and a mock whose specifier has a `?query` is a module id of its own; a
//! specifier with escaped quotes is read whole. All of it for setup-file mocks too, and an
//! `import()` waits for an async `vi.doMock` factory.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

/// Assert a clean pass with exactly `n` passing tests.
fn assert_passes(r: &common::Run, n: u32) {
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains(&format!("Tests  {n} passed ({n})")),
        "{}",
        r.all()
    );
}

/// A project without esbuild (the ESM-graph path).
fn esm_graph_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file(
        "package.json",
        &format!(r#"{{"name":"{name}","type":"module"}}"#),
    );
    p
}

#[test]
fn do_mock_then_dynamic_import_sees_the_mock_on_every_path() {
    // turbo-test#39 item 1: on the ESM graph (and the TURBO_NO_MR bundle path) vi.doMock had no
    // effect on a later import() — also when the real module was imported before.
    for esbuild in [false, true] {
        let p = if esbuild {
            let p = Project::new("rc-domock-import");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r39"}"#);
            p
        } else {
            esm_graph_project("rc-domock-import-esm")
        };
        p.file("m.js", "export const v = 'real';\n");
        p.file("r.js", "export const x = 'real-x';\n");
        p.file(
            "a.test.js",
            r#"import { x } from './r.js';
test('doMock then import()', async () => {
  vi.doMock('./m.js', () => ({ v: 'mocked' }));
  const m = await import('./m.js');
  expect(m.v).toBe('mocked');
});
test('doMock of an already-imported module, then import()', async () => {
  expect(x).toBe('real-x');
  vi.doMock('./r.js', () => ({ x: 'mocked-x' }));
  const r = await import('./r.js');
  expect(r.x).toBe('mocked-x');
});
"#,
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[&[], &[("TURBO_NO_MR", "1")]]
        } else {
            &[&[]]
        };
        for env in envs {
            assert_passes(&turbo_test(&p.dir, &["a.test.js"], env), 2);
        }
    }
}

#[test]
fn an_async_do_mock_factory_is_awaited_by_import() {
    // turbo-test#42 item 6: an async vi.doMock factory was queued, not awaited, so a following
    // import() resolved to the real module (ESM graph, TURBO_NO_MR) or hung until the test timed
    // out (module-runner). vitest awaits the factory; expected values are vitest 4.1.12's for the
    // same files.
    for esbuild in [false, true] {
        let p = if esbuild {
            let p = Project::new("rc-async-domock");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r42d"}"#);
            p
        } else {
            esm_graph_project("rc-async-domock-esm")
        };
        p.file("m.js", "export const v = 'real';\n");
        p.file("r.js", "export const x = 'real-x';\n");
        p.file("s.js", "export const w = 'real-w';\n");
        p.file("t.js", "export const t = 'real-t';\n");
        p.file("u.js", "export const u = 'real-u';\n");
        p.file("loader.js", "export const load = () => import('./u.js');\n");
        p.file(
            "a.test.js",
            r#"import { x } from './r.js';
test('async doMock factory, then import()', async () => {
  vi.doMock('./m.js', async () => ({ v: 'mocked' }));
  const m = await import('./m.js');
  expect(m.v).toBe('mocked');
});
test('a factory awaiting a timer, for a module imported before', async () => {
  expect(x).toBe('real-x');
  vi.doMock('./r.js', async () => {
    await new Promise((r) => setTimeout(r, 5));
    return { x: 'mocked-x' };
  });
  const r = await import('./r.js');
  expect(r.x).toBe('mocked-x');
});
test('a rejecting factory: import() rejects with its error', async () => {
  vi.doMock('./t.js', async () => {
    await new Promise((r) => setTimeout(r, 1));
    throw new Error('boom');
  });
  let err;
  try { await import('./t.js'); } catch (e) { err = e; }
  expect(String(err && err.message)).toContain('There was an error when mocking a module');
  expect(err.cause.message).toBe('boom');
});
test('an import() in another module waits too', async () => {
  vi.doMock('./u.js', async () => {
    await new Promise((r) => setTimeout(r, 1));
    return { u: 'mocked-u' };
  });
  const { load } = await import('./loader.js');
  expect((await load()).u).toBe('mocked-u');
});
test('a later doMock or doUnmock wins over a pending factory', async () => {
  vi.doMock('./m.js', async () => {
    await new Promise((r) => setTimeout(r, 1));
    return { v: 'stale' };
  });
  vi.doMock('./m.js', () => ({ v: 'sync' }));
  expect((await import('./m.js')).v).toBe('sync');
  await new Promise((r) => setTimeout(r, 5));
  expect((await import('./m.js')).v).toBe('sync');
  vi.doMock('./s.js', async () => {
    await new Promise((r) => setTimeout(r, 1));
    return { w: 'stale' };
  });
  vi.doUnmock('./s.js');
  await new Promise((r) => setTimeout(r, 5));
  expect((await import('./s.js')).w).toBe('real-w');
});
"#,
        );
        // importOriginal: a partial factory is not externalized on the TURBO_NO_MR bundle path
        // (a documented limitation of that escape hatch), so this one runs on the others only
        p.file(
            "b.test.js",
            r#"test('an async factory with importOriginal', async () => {
  vi.doMock('./s.js', async (importOriginal) => ({ ...(await importOriginal()), extra: 1 }));
  const s = await import('./s.js');
  expect([s.w, s.extra]).toEqual(['real-w', 1]);
});
"#,
        );
        assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 5);
        assert_passes(&turbo_test(&p.dir, &["b.test.js"], &[]), 1);
        if esbuild {
            assert_passes(
                &turbo_test(&p.dir, &["a.test.js"], &[("TURBO_NO_MR", "1")]),
                5,
            );
        }
    }
}

#[test]
fn a_query_suffixed_mock_is_a_module_of_its_own() {
    // turbo-test#39 item 2: resolve_spec strips `?query`, so `vi.mock('./rel.js?raw', ...)`
    // replaced the mock of `./rel.js`. vitest keys mocks by module id, query included.
    for esbuild in [true, false] {
        let p = if esbuild {
            let p = Project::new("rc-query-mock");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r39b"}"#);
            p
        } else {
            esm_graph_project("rc-query-mock-esm")
        };
        p.file("rel.js", "export const r = () => 'real';\n");
        p.file("other.js", "export const o = 'real-o';\n");
        p.file(
            "a.test.js",
            r#"import { r } from './rel.js';
import { o } from './other.js';
vi.mock('./rel.js', () => ({ r: () => 'mock-rel' }));
vi.mock('./rel.js?raw', () => ({ default: 'raw' }));
vi.mock('./other.js?inline', () => ({ default: 'inline' }));
test('query ids are separate modules', async () => {
  expect(r()).toBe('mock-rel');
  // only the query id is mocked: the plain module stays real
  expect(o).toBe('real-o');
  // the query id itself gets its own mock
  expect((await import('./rel.js?raw')).default).toBe('raw');
  expect((await import('./other.js?inline')).default).toBe('inline');
});
"#,
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[&[], &[("TURBO_NO_MR", "1")]]
        } else {
            &[&[]]
        };
        for env in envs {
            assert_passes(&turbo_test(&p.dir, &["a.test.js"], env), 1);
        }
    }
}

#[test]
fn a_setup_file_query_mock_is_a_module_of_its_own() {
    // turbo-test#42 item 5: turbo-test#39 item 2 externalized an entry file's `?query` mock
    // targets on the TURBO_NO_MR bundle path, but a setup file's were still skipped, so the
    // bundle inlined `./rel.js?raw` (a static import failed the graph, an `import()` got the
    // real module). Expected values are vitest 4.1.12's for the same files.
    for esbuild in [true, false] {
        let p = if esbuild {
            let p = Project::new("rc-setup-query-mock");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r42s"}"#);
            p
        } else {
            esm_graph_project("rc-setup-query-mock-esm")
        };
        p.file(
            "vitest.config.ts",
            "export default { test: { setupFiles: ['./setup.js'] } };\n",
        );
        p.file("rel.js", "export const r = () => 'real';\n");
        p.file("other.js", "export const o = 'real-o';\n");
        p.file(
            "setup.js",
            r#"vi.mock('./rel.js', () => ({ r: () => 'mock-rel' }));
vi.mock('./rel.js?raw', () => ({ default: 'raw' }));
vi.mock('./other.js?inline', () => ({ default: 'inline' }));
vi.mock('./missing.js?raw', () => ({ default: 'never imported' }));
vi.mock('not-installed', () => ({ default: 'never imported' }));
"#,
        );
        p.file(
            "a.test.js",
            r#"import { r } from './rel.js';
import { o } from './other.js';
import raw from './rel.js?raw';
test('setup-file query mocks are modules of their own', async () => {
  expect(r()).toBe('mock-rel');
  // only the query id is mocked: the plain module stays real
  expect(o).toBe('real-o');
  expect(raw).toBe('raw');
  expect((await import('./rel.js?raw')).default).toBe('raw');
  expect((await import('./other.js?inline')).default).toBe('inline');
});
"#,
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[&[], &[("TURBO_NO_MR", "1")]]
        } else {
            &[&[]]
        };
        for env in envs {
            assert_passes(&turbo_test(&p.dir, &["a.test.js"], env), 1);
        }
    }
}

#[test]
fn a_specifier_with_escaped_quotes_is_mocked() {
    // turbo-test#42 item 4: the mock-hoisting string scanners stopped at the first quote, so an
    // escaped quote (`"./w\"x.js"`, or esbuild's `require("./q\"a'b.js")` and template-literal
    // `vi.mock(`./q"a'b.js`)` for a specifier holding both quotes) truncated the specifier and
    // the mock missed its module. Expected values are vitest 4.1.12's for the same files.
    for esbuild in [true, false] {
        let p = if esbuild {
            let p = Project::new("rc-quoted-spec");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r42"}"#);
            p
        } else {
            esm_graph_project("rc-quoted-spec-esm")
        };
        p.file("q\"a'b.js", "export const f = () => 'real';\n");
        p.file("w\"x.js", "export const g = () => 'real-g';\n");
        p.file("w\"y.js", "export const h = () => 'real-h';\n");
        p.file(
            "a.test.js",
            r#"import { f } from './q"a\'b.js';
import { g } from "./w\"x.js";
import { h } from "./w\"y.js";
vi.mock('./q"a\'b.js', () => ({ f: () => 'mocked-f' }));
vi.mock("./w\"x.js", () => ({ g: () => 'mocked-g' }));
const mf = vi.mocked(f);
const mg = vi.mocked(g);
test('a mocked specifier with escaped quotes', () => {
  expect(mf()).toBe('mocked-f');
  expect(f()).toBe('mocked-f');
  expect(mg()).toBe('mocked-g');
  expect(g()).toBe('mocked-g');
  // a module whose specifier shares the mocked one's prefix up to the quote stays real
  expect(h()).toBe('real-h');
});
"#,
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[&[], &[("TURBO_NO_MR", "1")]]
        } else {
            &[&[]]
        };
        for env in envs {
            assert_passes(&turbo_test(&p.dir, &["a.test.js"], env), 1);
        }
    }
}
