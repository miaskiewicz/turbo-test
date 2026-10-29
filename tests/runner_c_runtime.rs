//! Runtime semantics that follow Node/vitest: the `URL` polyfill resolves relative references
//! like Node's WHATWG `URL`; an entry file's hoisted mock factory runs once; without esbuild the
//! entry prepass replays `vi.hoisted` values and awaits async factories; a setup file that throws
//! fails every test file using it; a passing matcher never formats its operands.
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

/// The URL cases: expected values captured from Node 26's `URL`.
const URL_TEST: &str = r##"// expected hrefs: Node 26 `new URL(input, base)` (TypeError = throws)
const CASES = [
  ["./x.json", "file:///d/a.test.ts", "file:///d/x.json"],
  ["../y/z.js", "file:///d/e/a.ts", "file:///d/y/z.js"],
  ["x", "file:///d/", "file:///d/x"],
  ["../../../../up", "file:///a/b/c", "file:///up"],
  ["/abs", "https://h.example/p/q?x=1", "https://h.example/abs"],
  ["?k=v", "https://h.example/p/q", "https://h.example/p/q?k=v"],
  ["#frag", "https://h.example/p/q?x=1#old", "https://h.example/p/q?x=1#frag"],
  ["", "https://h.example/p/q?x=1#old", "https://h.example/p/q?x=1"],
  ["//other.example/r", "https://h.example/p", "https://other.example/r"],
  ["https://abs.example/a/./b/../c", undefined, "https://abs.example/a/c"],
  ["a/./b/../c/", "http://h/base/", "http://h/base/a/c/"],
  ["..", "http://h/a/b/c", "http://h/a/"],
  [".", "http://h/a/b/c", "http://h/a/b/"],
  ["sub/file.txt?q=1#h", "http://h:8080/dir/index.html", "http://h:8080/dir/sub/file.txt?q=1#h"],
  ["#x", "mailto:me@example.com", "mailto:me@example.com#x"],
  ["y", "mailto:me@example.com", "TypeError"],
  ["./data/./x.json", "file:///proj/src/a.test.ts", "file:///proj/src/data/x.json"],
  ["http://h", undefined, "http://h/"],
  ["not a url", undefined, "TypeError"],
];
test('URL resolves like Node', () => {
  for (const [u, b, want] of CASES) {
    let got;
    try { got = (b === undefined ? new URL(u) : new URL(u, b)).href; } catch (e) { got = e instanceof TypeError ? 'TypeError' : String(e); }
    expect(`${u} @ ${b} -> ${got}`).toBe(`${u} @ ${b} -> ${want}`);
  }
  expect(new URL('./x.json', 'file:///d/a.test.ts').pathname).toBe('/d/x.json');
  // the import.meta.url pattern
  expect(new URL('./data.json', import.meta.url).href).toBe(import.meta.url.replace(/[^/]*$/, 'data.json'));
});
"##;

#[test]
fn url_resolves_relative_references_like_node() {
    // turbo-test#38 item 1: a relative reference was appended to the whole base
    // (`new URL('./x.json', 'file:///d/a.test.ts').pathname` == "/d/a.test.ts/./x.json").
    for esbuild in [false, true] {
        let p = Project::new("rc-url");
        if esbuild {
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r38"}"#);
        } else {
            p.file("package.json", r#"{"name":"r38","type":"module"}"#);
        }
        p.file("a.test.js", URL_TEST);
        assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
    }
}

#[test]
fn an_entry_mock_factory_runs_once() {
    // turbo-test#38 item 2: the entry's hoisted factories ran in the mock prepass AND again in
    // the module body; vitest calls a factory once. Sync, async and jest.mock factories, on the
    // module-runner, legacy-bundle (TURBO_NO_MR) and ESM-graph (no esbuild) paths.
    for esbuild in [true, false] {
        let p = Project::new("rc-factory-once");
        if esbuild {
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r38b"}"#);
            p.file(
                "node_modules/react/package.json",
                r#"{"name":"react","main":"index.js"}"#,
            );
            p.file(
                "node_modules/react/index.js",
                "exports.createElement = () => null;\n",
            );
        } else {
            p.file("package.json", r#"{"name":"r38b","type":"module"}"#);
        }
        p.file("m.js", "export const v = 'real';\n");
        p.file("n.js", "export const w = 'real-n';\n");
        p.file("j.js", "export const j = 'real-j';\n");
        // async factories without esbuild are turbo-test#38 item 5
        let n_factory = if esbuild { "async () =>" } else { "() =>" };
        p.file(
            "a.test.js",
            &format!(
                r#"import {{ v }} from './m.js';
import {{ w }} from './n.js';
import {{ j }} from './j.js';
vi.mock('./m.js', () => {{ globalThis.M = (globalThis.M || 0) + 1; return {{ v: 'mock' }}; }});
vi.mock('./n.js', {n_factory} {{ globalThis.N = (globalThis.N || 0) + 1; return {{ w: 'mock-n' }}; }});
jest.mock('./j.js', () => {{ globalThis.J = (globalThis.J || 0) + 1; return {{ j: 'mock-j' }}; }});
test('each factory ran once', () => {{
  expect([v, w, j]).toEqual(['mock', 'mock-n', 'mock-j']);
  expect([globalThis.M, globalThis.N, globalThis.J]).toEqual([1, 1, 1]);
}});
"#
            ),
        );
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[&[], &[("TURBO_NO_MR", "1")]]
        } else {
            &[&[]]
        };
        for env in envs {
            let r = turbo_test(&p.dir, &["a.test.js"], env);
            assert_passes(&r, 1);
        }
    }
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
fn esm_graph_entry_mock_factories_see_vi_hoisted_values() {
    // turbo-test#38 item 4: without esbuild, the entry's factories were compiled on their own,
    // without the file's vi.hoisted declarations ("calls is not defined").
    let p = esm_graph_project("rc-esm-hoisted");
    p.file(
        "m.js",
        "export const v = 'real';\nexport const u = 'real-u';\n",
    );
    p.file(
        "a.test.js",
        r#"import { v, u } from './m.js';
const calls = vi.hoisted(() => ({ m: 0 }));
const { label } = vi.hoisted(() => ({ label: 'hoisted' }));
vi.mock('./m.js', () => { calls.m++; return { v: label, u: 'mock-u' }; });
test('factory closes over vi.hoisted values', () => {
  expect(v).toBe('hoisted');
  expect(u).toBe('mock-u');
  expect(calls.m).toBe(1);
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn esm_graph_async_entry_mock_factories_are_awaited() {
    // turbo-test#38 item 5: without esbuild, an async factory's Promise was registered as the
    // module ("does not provide an export named 'w'").
    let p = esm_graph_project("rc-esm-async");
    p.file(
        "n.js",
        "export const w = 'real';\nexport default 'real-default';\n",
    );
    p.file(
        "o.js",
        "export const k = 'real-k';\nexport const keep = 'kept';\n",
    );
    p.file(
        "a.test.js",
        r#"import d, { w } from './n.js';
import { k, keep } from './o.js';
vi.mock('./n.js', async () => ({ w: 'mock', default: 'mock-default' }));
vi.mock('./o.js', async (importOriginal) => ({ ...(await importOriginal()), k: 'mock-k' }));
test('async factories are awaited', () => {
  expect([w, d]).toEqual(['mock', 'mock-default']);
  expect([k, keep]).toEqual(['mock-k', 'kept']);
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn esm_graph_throwing_vi_hoisted_fails_the_file_and_mocks_still_register() {
    // turbo-test#38 item 4 edge: a vi.hoisted that throws fails the file with its error
    // (vitest); the prepass registers the file's mocks without the hoisted values meanwhile.
    let p = esm_graph_project("rc-esm-hoisted-throw");
    p.file("m.js", "export const v = 'real';\n");
    p.file(
        "a.test.js",
        r#"import { v } from './m.js';
const h = vi.hoisted(() => { throw new Error('hoisted boom'); });
vi.mock('./m.js', () => ({ v: 'mock' }));
test('never runs', () => { expect(v).toBe('mock'); });
"#,
    );
    let r = turbo_test(&p.dir, &["a.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout
            .lines()
            .any(|l| l.starts_with("ERROR") && l.contains("hoisted boom")),
        "{}",
        r.all()
    );
}

#[test]
fn a_throwing_setup_file_fails_every_file_that_uses_it() {
    // turbo-test#38 item 3: a setup file that throws was ignored and the run passed. vitest fails
    // each test file with the setup file's error. Module-runner (also with isolate reuse, where
    // setup runs once per worker), TURBO_NO_MR (bundle path) and no esbuild.
    for esbuild in [true, false] {
        let p = if esbuild {
            let p = Project::new("rc-setup-throws");
            p.with_esbuild();
            p.file("package.json", r#"{"name":"r38c"}"#);
            p
        } else {
            esm_graph_project("rc-setup-throws-esm")
        };
        p.file(
            "vitest.config.js",
            "export default { test: { setupFiles: ['./setup.js'] } };\n",
        );
        p.file("setup.js", "throw new Error('setup boom');\n");
        p.file("a.test.js", "test('a', () => {});\n");
        p.file("b.test.js", "test('b', () => {});\n");
        let envs: &[&[(&str, &str)]] = if esbuild {
            &[
                &[],
                &[("TURBO_NO_MR", "1")],
                &[("TURBO_REUSE_ISOLATE", "1"), ("TURBO_JOBS", "1")],
            ]
        } else {
            &[&[], &[("TURBO_REUSE_ISOLATE", "1"), ("TURBO_JOBS", "1")]]
        };
        for env in envs {
            let r = turbo_test(&p.dir, &["a.test.js", "b.test.js"], env);
            assert_eq!(r.code, Some(1), "{env:?}\n{}", r.all());
            for f in ["a.test.js", "b.test.js"] {
                assert!(
                    r.stdout.lines().any(|l| l.starts_with("ERROR")
                        && l.contains(f)
                        && l.contains("setup.js")
                        && l.contains("setup boom")),
                    "{env:?} {f}\n{}",
                    r.all()
                );
            }
        }
    }
    // a setup file that works is unaffected
    let ok = Project::new("rc-setup-ok");
    ok.with_esbuild();
    ok.file("package.json", r#"{"name":"r38d"}"#);
    ok.file(
        "vitest.config.js",
        "export default { test: { setupFiles: ['./setup.js'] } };\n",
    );
    ok.file("setup.js", "globalThis.SETUP = 1;\n");
    ok.file(
        "a.test.js",
        "test('a', () => { expect(globalThis.SETUP).toBe(1); });\n",
    );
    for env in [&[][..], &[("TURBO_NO_MR", "1")][..]] {
        assert_passes(&turbo_test(&ok.dir, &["a.test.js"], env), 1);
    }
}

#[test]
fn a_setup_file_that_fails_to_link_fails_the_test_file() {
    // turbo-test#38 item 3, ESM graph: a setup file whose imports do not link (a missing named
    // export) fails the test file with the instantiate error.
    let p = esm_graph_project("rc-setup-link");
    p.file(
        "vitest.config.js",
        "export default { test: { setupFiles: ['./setup.js'] } };\n",
    );
    p.file("m.js", "export const v = 1;\n");
    p.file(
        "setup.js",
        "import { nope } from './m.js';\nglobalThis.N = nope;\n",
    );
    p.file("a.test.js", "test('a', () => {});\n");
    let r = turbo_test(&p.dir, &["a.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.lines().any(|l| l.starts_with("ERROR")
            && l.contains("setup file")
            && l.contains("instantiate:")
            && l.contains("nope")),
        "{}",
        r.all()
    );
}

/// turbo-test#43 item 2: a passing matcher never formats its operands (vitest builds the
/// failure message lazily); an operand whose JSON/string conversion throws — like a native
/// class prototype whose members check their receiver — fails only a failing assertion. The
/// spec passes under vitest 4.1.12 (matchers that vitest itself formats or deep-compares on a
/// pass, such as toContain and toHaveBeenCalledWith, are left out).
const LAZY_MESSAGE_TEST: &str = r#"import { test, expect, vi } from 'vitest';
const bad = () => { throw new TypeError('Illegal invocation'); };
const h = { toString: bad, valueOf: bad, toJSON: bad, [Symbol.toPrimitive]: bad };
Object.defineProperty(h, 'x', { get: bad, enumerable: true });
const spy = vi.fn();
spy(h);
const cases = {
  toBe: () => expect(h).toBe(h),
  toBeTruthy: () => expect(h).toBeTruthy(),
  notToBeFalsy: () => expect(h).not.toBeFalsy(),
  notToBeNull: () => expect(h).not.toBeNull(),
  notToBeUndefined: () => expect(h).not.toBeUndefined(),
  toBeDefined: () => expect(h).toBeDefined(),
  toBeTypeOf: () => expect(h).toBeTypeOf('object'),
  toSatisfy: () => expect(h).toSatisfy(() => true),
  toBeInstanceOf: () => expect(h).toBeInstanceOf(Object),
  toHaveProperty: () => expect(h).toHaveProperty('toString'),
  toHaveBeenLastCalledWith: () => expect(spy).toHaveBeenLastCalledWith(h),
  toHaveBeenNthCalledWith: () => expect(spy).toHaveBeenNthCalledWith(1, h),
  toHaveReturnedWith: () => expect(spy).toHaveReturnedWith(undefined),
  toHaveBeenCalled: () => expect(spy).toHaveBeenCalled(),
  toEqual: () => expect(h).toEqual(h),
  toStrictEqual: () => expect(h).toStrictEqual(h),
  toMatchObject: () => expect(h).toMatchObject({}),
};
for (const [name, fn] of Object.entries(cases)) test(name, fn);
test('a failing matcher still formats its message', () => {
  expect(() => expect(1).toBe(2)).toThrow('expected 1 to be 2');
  expect(() => expect([1]).toHaveLength(2)).toThrow('expected');
  expect(() => expect(h).toBe(1)).toThrow('Illegal invocation');
});
"#;

#[test]
fn passing_matchers_do_not_format_their_operands() {
    let p = Project::new("rc-lazy-msg");
    p.file("lazy.test.ts", LAZY_MESSAGE_TEST);
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 18);
}

/// turbo-test#46: structuredClone is the HTML structured clone (V8's serializer, as Node's):
/// types, cycles and shared references survive, `transfer` moves ArrayBuffers (the originals
/// are detached), SharedArrayBuffers are shared, and what cannot be cloned or transferred
/// throws Node's DataCloneError. The spec passes under vitest 4.1.12.
const STRUCTURED_CLONE_TEST: &str = r#"import { test, expect } from 'vitest';

// expected values: vitest 4.1.12 (Node 26.11's structuredClone)
const err = (f: () => unknown) => {
  try { f(); } catch (e: any) { return [e.constructor.name, e.name, e.message]; }
  return 'no throw';
};

test('values keep their types; cycles and shared references survive', () => {
  const o: any = { d: new Date(5), m: new Map([[1, new Set(['a'])]]), r: /x/gi, big: 10n, u: undefined, n: null };
  o.self = o;
  o.twice = [o.d, o.d];
  const c = structuredClone(o);
  expect(c).not.toBe(o);
  expect(c.self).toBe(c);
  expect(c.d).toBeInstanceOf(Date);
  expect(c.d.getTime()).toBe(5);
  expect(c.twice[0]).toBe(c.twice[1]);
  expect(c.m.get(1).has('a')).toBe(true);
  expect([c.r.source, c.r.flags, c.big, 'u' in c, c.n]).toEqual(['x', 'gi', 10n, true, null]);
  const e = structuredClone(new RangeError('boom'));
  expect([e instanceof RangeError, e.message]).toEqual([true, 'boom']);
  const u8 = new Uint8Array([1, 2, 3]);
  const cu = structuredClone(u8);
  expect([cu instanceof Uint8Array, [...cu], cu.buffer === u8.buffer]).toEqual([true, [1, 2, 3], false]);
});

test('transfer moves ArrayBuffers; SharedArrayBuffers are shared', () => {
  const b = new ArrayBuffer(4);
  new Uint8Array(b)[0] = 9;
  const c = structuredClone(b, { transfer: [b] });
  expect([b.byteLength, c.byteLength, new Uint8Array(c)[0]]).toEqual([0, 4, 9]);
  const u = new Uint8Array([1, 2, 3]);
  const r = structuredClone({ u }, { transfer: [u.buffer] });
  expect([u.byteLength, [...r.u]]).toEqual([0, [1, 2, 3]]);
  const s = new SharedArrayBuffer(4);
  new Uint8Array(structuredClone(s))[0] = 7;
  expect(new Uint8Array(s)[0]).toBe(7);
  expect(structuredClone(1, { transfer: undefined })).toBe(1);
  expect(structuredClone(2, null as any)).toBe(2);
});

test('what cannot be cloned or transferred throws as Node', () => {
  const ab = new ArrayBuffer(2);
  expect(err(() => structuredClone(1, { transfer: [{} as any] }))).toEqual(['DOMException', 'DataCloneError', 'Found invalid value in transferList.']);
  expect(err(() => structuredClone(1, { transfer: [ab, ab] }))).toEqual(['DOMException', 'DataCloneError', 'Transfer list contains duplicate ArrayBuffer']);
  expect(err(() => structuredClone(() => 1))).toEqual(['DOMException', 'DataCloneError', '() => 1 could not be cloned.']);
  expect(err(() => structuredClone(Symbol()))).toEqual(['DOMException', 'DataCloneError', 'Symbol() could not be cloned.']);
  expect(err(() => (structuredClone as any)())).toEqual(['TypeError', 'TypeError', 'The "The value argument must be specified" argument must be specified']);
  expect(err(() => structuredClone(1, { transfer: 5 as any }))).toEqual(['TypeError', 'TypeError', "Failed to execute 'structuredClone': transfer in Options cannot be converted to sequence."]);
  expect(err(() => structuredClone(1, { transfer: [new SharedArrayBuffer(1) as any] }))).toEqual(['DOMException', 'DataCloneError', 'Found invalid value in transferList.']);
  const mem = new WebAssembly.Memory({ initial: 1 });
  expect(err(() => structuredClone(1, { transfer: [mem.buffer] }))).toEqual(['DOMException', 'DataCloneError', 'Cannot transfer object of unsupported type.']);
  const d = new ArrayBuffer(1);
  structuredClone(d, { transfer: [d] });
  expect(err(() => structuredClone(d, { transfer: [d] }))).toEqual(['DOMException', 'DataCloneError', 'Cannot transfer object of unsupported type.']);
  expect(err(() => structuredClone(d))).toEqual(['DOMException', 'DataCloneError', 'An ArrayBuffer is detached and could not be cloned.']);
});
"#;

#[test]
fn structured_clone_follows_node() {
    for env in ["node", "jsdom"] {
        let p = Project::new("rc-structured-clone");
        p.file("sc.test.ts", STRUCTURED_CLONE_TEST);
        let r = turbo_test(&p.dir, &["--environment", env], &[]);
        assert_passes(&r, 3);
    }
}

/// turbo-test#48: fs.symlinkSync and fs.promises.symlink create symbolic links; a failure is
/// Node's error (code, syscall, message). The spec passes under vitest 4.1.12.
const SYMLINK_TEST: &str = r#"import { test, expect } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';

// expected values: vitest 4.1.12 (Node 26.11)
test('fs.symlinkSync / fs.promises.symlink create symbolic links', async () => {
  const dir = `${new URL(".", import.meta.url).pathname}sym-${Date.now()}`;
  fs.mkdirSync(dir);
  fs.writeFileSync(`${dir}/target.txt`, 'through the link');
  expect(fs.symlinkSync(`${dir}/target.txt`, `${dir}/link.txt`)).toBe(undefined);
  expect(fs.readFileSync(`${dir}/link.txt`, 'utf8')).toBe('through the link');
  let err: any;
  try { fs.symlinkSync(`${dir}/target.txt`, `${dir}/link.txt`); } catch (e) { err = e; }
  expect([err.code, err.syscall, err.message]).toEqual([
    'EEXIST', 'symlink', `EEXIST: file already exists, symlink '${dir}/target.txt' -> '${dir}/link.txt'`,
  ]);
  await fs.promises.symlink('target.txt', `${dir}/relative.txt`);
  expect(fs.readFileSync(`${dir}/relative.txt`, 'utf8')).toBe('through the link');
  let err2: any;
  await fs.promises.symlink('x', `${dir}/missing/link`).catch((e) => { err2 = e; });
  expect(err2.code).toBe('ENOENT');
  fs.rmSync(dir, { recursive: true });
  void os;
});
"#;

#[test]
fn fs_symlink_creates_links() {
    let p = Project::new("rc-symlink");
    p.with_esbuild();
    p.file("sym.test.ts", SYMLINK_TEST);
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}

/// turbo-test#54: process.cwd() is the process's working directory (vitest 4.1.12: the
/// directory it was started in, also below the project root), not the package root.
#[test]
fn process_cwd_is_the_working_directory() {
    let p = Project::new("rc-cwd");
    p.file("package.json", "{}");
    p.file(
        "sub/cwd.test.ts",
        "import { test } from 'vitest';\ntest('cwd', () => { console.log('CWD=' + process.cwd()); });\n",
    );
    let sub = p.dir.join("sub");
    let r = turbo_test(&sub, &[], &[]);
    assert_passes(&r, 1);
    let want = format!("CWD={}", sub.canonicalize().unwrap().display());
    let got = r.all();
    assert!(
        got.contains(&want) || got.contains(&format!("CWD={}", sub.display())),
        "{want}\n{got}"
    );
}

/// turbo-test#55: os.tmpdir() / os.homedir() are the environment's, as vitest 4.1.12 (Node):
/// TMPDIR without its trailing slash, HOME.
#[test]
fn os_tmpdir_and_homedir_follow_the_environment() {
    let p = Project::new("rc-os-dirs");
    p.file(
        "os.test.ts",
        "import { test, expect } from 'vitest';\nimport * as os from 'os';\n\
         test('dirs', () => {\n  expect(os.tmpdir()).toBe('/tt/tmp/dir');\n  expect(os.homedir()).toBe('/tt/home');\n});\n",
    );
    let r = turbo_test(
        &p.dir,
        &[],
        &[("TMPDIR", "/tt/tmp/dir/"), ("HOME", "/tt/home")],
    );
    assert_passes(&r, 1);
}

/// turbo-test#56: require.resolve gives the resolved path (a builtin its name), and require /
/// require.resolve of a module that cannot be found throw Node's MODULE_NOT_FOUND with the
/// require stack. The spec passes under vitest 4.1.12.
const REQUIRE_RESOLVE_TEST: &str = r#"import { test, expect } from 'vitest';
import * as fs from 'fs';

// expected values: vitest 4.1.12 (Node's require)
const dir = new URL('.', import.meta.url).pathname;
const here = dir + 'resolve.test.ts';
fs.writeFileSync(dir + 'res-m.js', 'module.exports = 1;');
fs.writeFileSync(dir + 'res-inner.js', "module.exports = () => require('./res-missing');");

test('require.resolve gives the resolved path; a builtin stays its name', () => {
  expect(require.resolve('./res-m.js')).toBe(dir + 'res-m.js');
  expect(require.resolve('./res-m')).toBe(dir + 'res-m.js');
  expect([require.resolve('fs'), require.resolve('node:path')]).toEqual(['fs', 'node:path']);
});

test('a module that cannot be found is MODULE_NOT_FOUND, with the require stack', () => {
  const caught = (f: () => unknown) => { try { f(); } catch (e: any) { return [e.code, e.message]; } return 'no throw'; };
  const want = ['MODULE_NOT_FOUND', `Cannot find module './res-nope'\nRequire stack:\n- ${here}`];
  expect(caught(() => require.resolve('./res-nope'))).toEqual(want);
  expect(caught(() => require('./res-nope'))).toEqual(want);
  expect(caught(() => require('./res-inner.js')())).toEqual([
    'MODULE_NOT_FOUND', `Cannot find module './res-missing'\nRequire stack:\n- ${dir}res-inner.js\n- ${here}`,
  ]);
});
"#;

#[test]
fn require_resolve_follows_node() {
    let p = Project::new("rc-require-resolve");
    p.with_esbuild();
    p.file("resolve.test.ts", REQUIRE_RESOLVE_TEST);
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 2);
}

/// turbo-test#47: require.cache is the shared module cache (Node's Module._cache): entries by
/// path, and deleting one makes the next require() load the module again. The spec passes under
/// vitest 4.1.12.
const REQUIRE_CACHE_TEST: &str = r#"import { test, expect } from 'vitest';
import * as fs from 'fs';

// expected values: vitest 4.1.12 (Node's require.cache)
const dir = new URL('.', import.meta.url).pathname;
fs.writeFileSync(dir + 'cache-inj.js', "module.exports = 'real';");
fs.writeFileSync(dir + 'cache-m.js', 'globalThis.__cacheRuns = (globalThis.__cacheRuns || 0) + 1; module.exports = { run: globalThis.__cacheRuns };');

test('require.cache holds the loaded module; deleting it loads the module again', () => {
  const a = require('./cache-m.js');
  const key = require.resolve('./cache-m.js');
  const entry = require.cache[key]!;
  expect([entry.exports === a, entry.id, entry.filename, entry.loaded]).toEqual([true, key, key, true]);
  expect(require.cache[key]).toBe(entry);
  expect(key in require.cache).toBe(true);
  expect(Object.keys(require.cache)).toContain(key);
  expect(require('./cache-m.js')).toBe(a);
  expect(delete require.cache[key]).toBe(true);
  expect([key in require.cache, require.cache[key]]).toEqual([false, undefined]);
  const b = require('./cache-m.js');
  expect(b).not.toBe(a);
  expect(b.run).toBe(a.run + 1);
  expect(require.cache[key]!.exports).toBe(b);
});

test('a module put into require.cache is what require() gives', () => {
  const key = require.resolve('./cache-inj.js');
  require.cache[key] = { exports: 'injected' } as any;
  expect(require('./cache-inj.js')).toBe('injected');
  require.cache[key]!.exports = 'replaced';
  expect(require('./cache-inj.js')).toBe('replaced');
});
"#;

#[test]
fn require_cache_deletion_loads_the_module_again() {
    let p = Project::new("rc-require-cache");
    p.with_esbuild();
    p.file("cache.test.ts", REQUIRE_CACHE_TEST);
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 2);
}

#[test]
fn process_stdout_and_stderr_write_reach_the_streams() {
    // turbo-test#70: process.stdout.write / process.stderr.write were no-ops. As in Node, the text
    // reaches stdout / stderr unprefixed; `write` returns true and calls a trailing callback.
    // stdout text moves to stderr while a JSON report owns stdout (like console output).
    let p = Project::new("rc-stdio");
    p.file(
        "w.test.js",
        "test('w', async () => {\n\
           let called = 0\n\
           expect(process.stdout.write('OUT1\\n')).toBe(true)\n\
           expect(process.stderr.write(new TextEncoder().encode('ERR1\\n'), () => { called++ })).toBe(true)\n\
           process.stdout.write('OUT2', 'utf8', () => { called++ })\n\
           process.stdout.write('\\n')\n\
           await new Promise((r) => setTimeout(r, 0))\n\
           expect(called).toBe(2)\n\
         })\n",
    );
    let r = turbo_test(&p.dir, &["w.test.js"], &[]);
    assert_passes(&r, 1);
    assert!(r.stdout.starts_with("OUT1\nOUT2\n"), "{}", r.all());
    assert!(
        r.stderr.contains("ERR1\n") && !r.stdout.contains("ERR1"),
        "{}",
        r.all()
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "w.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.starts_with('{'), "{}", r.all());
    // one stream now: the writes in order
    assert!(r.stderr.starts_with("OUT1\nERR1\nOUT2\n"), "{}", r.all());
    let r = turbo_test(&p.dir, &["--silent", "w.test.js"], &[]);
    assert_passes(&r, 1);
    assert!(
        !r.stdout.contains("OUT1") && !r.stderr.contains("ERR1"),
        "{}",
        r.all()
    );
}

/// turbo-test#85: `URL` percent-encodes the path and the fragment with the WHATWG sets (values:
/// Node 26 `new URL(...)`).
#[test]
fn url_percent_encodes_the_path_and_fragment_like_node() {
    let p = Project::new("rc-url-encode");
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "a.test.js",
        r#"import { expect, it } from 'vitest';
it('percent-encodes', () => {
  const parts = (s) => { const u = new URL(s); return [u.href, u.pathname, u.hash]; };
  expect(parts('file:///a b')).toEqual(['file:///a%20b', '/a%20b', '']);
  expect(parts('http://h/a b#e f')).toEqual(['http://h/a%20b#e%20f', '/a%20b', '#e%20f']);
  expect(parts('http://h/x"<>`{}|^')).toEqual(['http://h/x%22%3C%3E%60%7B%7D|%5E', '/x%22%3C%3E%60%7B%7D|%5E', '']);
  expect(parts('http://h/€#€')).toEqual(['http://h/%E2%82%AC#%E2%82%AC', '/%E2%82%AC', '#%E2%82%AC']);
  expect(parts('http://h/%41%zz#%20')).toEqual(['http://h/%41%zz#%20', '/%41%zz', '#%20']);
  expect(new URL('./c d', 'file:///p q/r').href).toBe('file:///p%20q/c%20d');
  const u = new URL('http://h/x');
  u.pathname = '/n m';
  expect(u.href).toBe('http://h/n%20m');
  const f = new URL('http://h/x#');
  expect([f.href, f.hash]).toEqual(['http://h/x#', '']);
  f.hash = 'a b';
  expect(f.href).toBe('http://h/x#a%20b');
  f.hash = '';
  expect(f.href).toBe('http://h/x');
  expect([new URL('  http://h/a\tb\n ').href, new URL('mailto:a b\x01').href, new URL('mailto:x\x01y').href]).toEqual(['http://h/ab', 'mailto:a b', 'mailto:x%01y']);
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &[], &[]), 1);
}

/// turbo-test#84: `node:url` `fileURLToPath` decodes and validates like Node, `pathToFileURL`
/// returns an encoded `URL` (values: Node 26; the host error names `process.platform`, as Node does).
#[test]
fn file_url_conversions_follow_node() {
    let p = Project::new("rc-file-url");
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "a.test.js",
        r#"import { expect, it } from 'vitest';
import { fileURLToPath, pathToFileURL } from 'node:url';
import * as url from 'url';
it('fileURLToPath', () => {
  expect(['file:///a%20b', 'file:///a/b%23c?x#y', 'file://localhost/x', 'file:///%E2%82%AC'].map((s) => fileURLToPath(s))).toEqual(['/a b', '/a/b#c', '/x', '/€']);
  expect(fileURLToPath(new URL('file:///u%20v'))).toBe('/u v');
  const err = (s) => { try { fileURLToPath(s); return 'no throw'; } catch (e) { return [e.name, e.code, e.message]; } };
  expect(err('http://x/y')).toEqual(['TypeError', 'ERR_INVALID_URL_SCHEME', 'The URL must be of scheme file']);
  expect(err('file:///a%2Fb')).toEqual(['TypeError', 'ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded / characters']);
  expect(err('file://host/x')).toEqual(['TypeError', 'ERR_INVALID_FILE_URL_HOST', `File URL host must be "localhost" or empty on ${process.platform}`]);
  expect(url.fileURLToPath('file:///a%20b')).toBe('/a b');
  const local = new URL('file://LocalHost/x');
  expect([local.href, local.host, local.origin]).toEqual(['file:///x', '', 'null']);
});
it('pathToFileURL', () => {
  const u = pathToFileURL('/a b#c?d%e');
  expect([u instanceof URL, u.href, u.pathname]).toEqual([true, 'file:///a%20b%23c%3Fd%25e', '/a%20b%23c%3Fd%25e']);
  expect(pathToFileURL('/x/€/y').href).toBe('file:///x/%E2%82%AC/y');
  expect(pathToFileURL('/t\tn').href).toBe('file:///t%09n');
  expect(pathToFileURL('rel/x').href).toBe('file://' + process.cwd() + '/rel/x');
  expect(pathToFileURL('/d/').href).toBe('file:///d/');
  expect([pathToFileURL('').href, pathToFileURL('.').href, pathToFileURL('./').href]).toEqual(['file://' + process.cwd(), 'file://' + process.cwd(), 'file://' + process.cwd() + '/']);
  expect(url.pathToFileURL('/q r').href).toBe('file:///q%20r');
  expect(fileURLToPath(pathToFileURL('/r t/€'))).toBe('/r t/€');
  let ascii = '';
  for (let c = 0x20; c < 0x7f; c++) if (c !== 0x2f) ascii += String.fromCharCode(c);
  expect(pathToFileURL('/' + ascii).href).toBe("file:///%20!%22%23$%25&'()*+,-.0123456789:;%3C=%3E%3F@ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D%7E");
  expect(pathToFileURL('/a\x01\x1f\x7fb').href).toBe('file:///a%01%1F%7Fb');
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &[], &[]), 2);
}

#[test]
fn an_imported_markdown_file_is_parsed_as_javascript() {
    // `.md` is no vite asset type: vite parses it as JavaScript, so importing one fails the file
    // with rolldown's parse failure (vitest 4.1.12's message for this project), on both loaders
    for esbuild in [false, true] {
        let p = Project::new(if esbuild { "rc-md-mr" } else { "rc-md-graph" });
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{"name":"rc-md","type":"module"}"#);
        p.file("notes.md", "# Title\n\nSome *text*.\n");
        p.file(
            "a.test.js",
            "import { test } from 'vitest';\nimport notes from './notes.md';\ntest('a', () => { notes; });\n",
        );
        let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
        let report = common::json_report(&r);
        let file = &report["testResults"][0];
        assert_eq!(
            common::file_status(file),
            "error",
            "esbuild={esbuild}\n{}",
            r.all()
        );
        if !esbuild {
            assert_eq!(
                file["message"],
                "Parse failure: Parse failed with 1 error:\nInvalid Character ` `\n1: # Title\n    ^\n2: \n3: Some *text*.\nAt file: /notes.md:1:1",
                "{}",
                r.all()
            );
        }
    }
}

/// turbo-test#199: on Windows the `node:url` helpers, `import.meta.url` and `import.meta.resolve`
/// take native drive-letter / UNC paths, like Node's (values: Node 26 `{ windows: true }`). The
/// runtime reads `process.platform` at call time, so the win32 branch runs on every OS here.
#[test]
fn file_url_helpers_handle_windows_paths() {
    let p = Project::new("rc-file-url-win32");
    p.file("package.json", r#"{"type":"module"}"#);
    p.file(
        "a.test.js",
        r#"import { expect, it, afterEach } from 'vitest';
import { fileURLToPath, pathToFileURL } from 'node:url';
const platform = globalThis.__ttPlatform;
const cwd = process.cwd;
const requireResolve = globalThis.__ttRequireResolve;
afterEach(() => { globalThis.__ttPlatform = platform; process.cwd = cwd; globalThis.__ttRequireResolve = requireResolve; });
const win = () => { globalThis.__ttPlatform = 'win32'; process.cwd = () => 'D:\\work\\proj'; };
it('pathToFileURL', () => {
  win();
  expect(['C:\\proj\\a b.test.ts', 'C:/x/y#z', '\\\\srv\\share\\f.ts', 'C:\\a\\..\\b\\', 'c:\\€\\%', 'rel\\x.ts', '\\top\\y'].map((s) => pathToFileURL(s).href))
    .toEqual(['file:///C:/proj/a%20b.test.ts', 'file:///C:/x/y%23z', 'file://srv/share/f.ts', 'file:///C:/b/', 'file:///c:/%E2%82%AC/%25', 'file:///D:/work/proj/rel/x.ts', 'file:///D:/top/y']);
  expect([pathToFileURL('').href, pathToFileURL('sub\\').href]).toEqual(['file:///D:/work/proj', 'file:///D:/work/proj/sub/']);
});
it('fileURLToPath', () => {
  win();
  expect(['file:///C:/a%20b/c.ts', 'file://srv/share/x', 'file:///c:/'].map((s) => fileURLToPath(s))).toEqual(['C:\\a b\\c.ts', '\\\\srv\\share\\x', 'c:\\']);
  const err = (s) => { try { fileURLToPath(s); return 'no throw'; } catch (e) { return [e.code, e.message]; } };
  expect(err('file:///a')).toEqual(['ERR_INVALID_FILE_URL_PATH', 'File URL path must be absolute']);
  expect(err('file:///C:/a%5Cb')).toEqual(['ERR_INVALID_FILE_URL_PATH', 'File URL path must not include encoded \\ or / characters']);
});
it('import.meta.resolve', () => {
  win();
  const seen = [];
  globalThis.__ttRequireResolve = (dir, spec) => { seen.push(dir); return spec === 'fs' ? 'node:fs' : 'C:\\proj\\node_modules\\pkg\\index.js'; };
  expect(import.meta.resolve('pkg', 'file:///C:/proj/src/a.js')).toBe('file:///C:/proj/node_modules/pkg/index.js');
  expect(import.meta.resolve('fs', 'file:///C:/proj/src/a.js')).toBe('node:fs');
  expect(seen).toEqual(['C:\\proj\\src', 'C:\\proj\\src']);
});
it('posix is unchanged', () => {
  expect(pathToFileURL('/a b').href).toBe('file:///a%20b');
  expect(pathToFileURL('C:\\x').href).toBe('file://' + process.cwd() + '/C:%5Cx');
  expect(fileURLToPath('file:///C:/a')).toBe('/C:/a');
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &[], &[]), 4);
}

#[test]
fn url_host_hostname_and_port_setters_update_href_like_node() {
    // turbo-test#201: host/hostname/port were plain fields; href and origin ignored a set.
    // Rows: [base, field, value, href, origin, host, hostname, port] from Node 26.
    let test = r##"const ROWS = [["file:///","hostname","srv","file://srv/","null","srv","srv",""],["file:///","host","a","file://a/","null","a","a",""],["http://a.com:8080/p?q#h","hostname","b.org","http://b.org:8080/p?q#h","http://b.org:8080","b.org:8080","b.org","8080"],["http://a.com:8080/p?q#h","host","c.net:81","http://c.net:81/p?q#h","http://c.net:81","c.net:81","c.net","81"],["http://a.com:8080/p","host","c.net","http://c.net:8080/p","http://c.net:8080","c.net:8080","c.net","8080"],["http://a.com/p","port","9000","http://a.com:9000/p","http://a.com:9000","a.com:9000","a.com","9000"],["http://a.com:9000/p","port","","http://a.com/p","http://a.com","a.com","a.com",""],["http://a.com/p","port","80","http://a.com/p","http://a.com","a.com","a.com",""],["https://a.com/p","port","443","https://a.com/p","https://a.com","a.com","a.com",""],["http://a.com/p","port","12abc","http://a.com:12/p","http://a.com:12","a.com:12","a.com","12"],["http://a.com/p","port","x","http://a.com/p","http://a.com","a.com","a.com",""],["http://a.com/p","hostname","B.ORG","http://b.org/p","http://b.org","b.org","b.org",""],["http://u:pw@a.com/p","host","z.io:7","http://u:pw@z.io:7/p","http://z.io:7","z.io:7","z.io","7"],["http://a.com/p","host","","http://a.com/p","http://a.com","a.com","a.com",""],["file:///x","port","5","file:///x","null","","",""],["http://a.com:1/p","hostname","q:9","http://a.com:1/p","http://a.com:1","a.com:1","a.com","1"]];
test('URL host setters', () => {
  for (const [base, field, v, ...want] of ROWS) {
    const u = new URL(base);
    u[field] = v;
    expect(`${base} .${field}=${v} -> ${[u.href, u.origin, u.host, u.hostname, u.port]}`)
      .toBe(`${base} .${field}=${v} -> ${want}`);
    expect(String(u)).toBe(u.href);
    expect(new URL(u.href).href).toBe(u.href);
  }
});
"##;
    let p = Project::new("rc-url-host");
    p.file("package.json", r#"{"name":"r201","type":"module"}"#);
    p.file("a.test.js", test);
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn url_scheme_userinfo_setters_and_invalid_hosts_match_node() {
    // RV3: the scheme kept its case ("HTTP:"), protocol/username/password were plain fields
    // (a set skipped validation, encoding and the default-port drop), and an out-of-range port
    // or a forbidden host code point parsed instead of throwing.
    // Rows: [base, field|null, value, href, protocol, username, password, origin] from Node 26
    // ("THROW": new URL(base) throws a TypeError).
    let test = r##"const ROWS = [["HTTP://Example.COM:80/a",null,null,"http://example.com/a","http:","","","http://example.com"],["FiLe:///x",null,null,"file:///x","file:","","","null"],["Foo://Ab/x",null,null,"foo://Ab/x","foo:","","","null"],["http://a:65536/",null,null,"THROW"],["http://a b/",null,null,"THROW"],["http://a<b/",null,null,"THROW"],["http://a:8x/",null,null,"THROW"],["http://a/","protocol","https","https://a/","https:","","","https://a"],["http://a/","protocol","HTTPS:","https://a/","https:","","","https://a"],["https://a:8080/","protocol","http:","http://a:8080/","http:","","","http://a:8080"],["https://a:80/x","protocol","http","http://a/x","http:","","","http://a"],["http://a/","protocol","foo","http://a/","http:","","","http://a"],["foo://a/","protocol","http","foo://a/","foo:","","","null"],["http://a/","protocol","1x","http://a/","http:","","","http://a"],["http://a/","protocol","wss:x","wss://a/","wss:","","","wss://a"],["http://u@a/","protocol","file","http://u@a/","http:","u","","http://a"],["http://a/","username","x y","http://x%20y@a/","http:","x%20y","","http://a"],["http://a/","password","p@ss:w","http://:p%40ss%3Aw@a/","http:","","p%40ss%3Aw","http://a"],["http://u:p@a/","username","","http://:p@a/","http:","","p","http://a"],["file:///x","username","u","file:///x","file:","","","null"],["mailto:x","password","p","mailto:x","mailto:","","","null"]];
test('URL scheme and userinfo', () => {
  for (const [base, field, v, ...want] of ROWS) {
    let got;
    try {
      const u = new URL(base);
      if (field) u[field] = v;
      got = [u.href, u.protocol, u.username, u.password, u.origin];
    } catch (e) { got = [e instanceof TypeError ? 'THROW' : String(e)]; }
    expect(`${base} .${field}=${v} -> ${got}`).toBe(`${base} .${field}=${v} -> ${want}`);
  }
});
"##;
    let p = Project::new("rc-url-scheme");
    p.file("package.json", r#"{"name":"rv3url","type":"module"}"#);
    p.file("a.test.js", test);
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn node_path_posix_ops_match_node() {
    // RV3: the node:path shim's join/normalize kept dot segments, resolve ignored a later
    // absolute segment, relative returned its second argument, and format/basename/extname
    // missed Node's edge cases. Rows: [fn, args, result] from Node 26 (absolute paths only,
    // so the result does not depend on the cwd).
    let test = r##"import path from 'node:path';
const ROWS = [["join",["a","..","..","b"],"../b"],["join",["/a/","./b","../c/"],"/a/c/"],["join",[""],"."],["join",["a","","/b"],"a/b"],["normalize",["a//b/./c/.."],"a/b"],["normalize",["/../x"],"/x"],["normalize",[""],"."],["normalize",["./"],"./"],["normalize",["a/.."],"."],["normalize",["//a//"],"/a/"],["resolve",["/a","/b","c"],"/b/c"],["resolve",["/a/b","../c","./d/"],"/a/c/d"],["resolve",["/x",""],"/x"],["relative",["/a/b/c","/a/d"],"../../d"],["relative",["/a","/a"],""],["relative",["/a/b","/a/b/c/d"],"c/d"],["relative",["/","/x"],"x"],["relative",["/x/y","/"],"../.."],["relative",["/foo/bar","/foo/barbaz"],"../barbaz"],["format",[{"root":"/","name":"f","ext":"txt"}],"/f.txt"],["format",[{"dir":"/d","base":"b"}],"/d/b"],["basename",["/a/b/"],"b"],["basename",["/a/b.js","b.js"],"b.js"],["extname",[".bashrc"],""],["extname",["a."],"."],["dirname",["a/b/"],"a"],["parse",["/x/.rc"],{"root":"/","dir":"/x","base":".rc","ext":"","name":".rc"}]];
test('node:path', () => {
  for (const [f, args, want] of ROWS) {
    expect(`${f}(${JSON.stringify(args)}) -> ${JSON.stringify(path[f](...args))}`)
      .toBe(`${f}(${JSON.stringify(args)}) -> ${JSON.stringify(want)}`);
    expect(path.posix[f](...args)).toEqual(want);
  }
});
"##;
    let p = Project::new("rc-node-path");
    p.file("package.json", r#"{"name":"rv3path","type":"module"}"#);
    p.file("a.test.js", test);
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}
