//! The module-runner path (an esbuild project: the entry and its require-graph load per module
//! as CommonJS): vi.mock factories (sync/async/partial/automock), importActual, doMock/unmock,
//! resetModules, the native `require` (builtins, `vitest`, JSON/CSS, packages, addons, cycles,
//! load failures), e2e-only stubbing of broken dependencies, lazy barrel stubs, decorator
//! metadata retry and the CJS loader's diagnostics.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn esbuild_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p
}

fn pkg(p: &Project, name: &str, main: &str, src: &str) {
    p.file(
        &format!("node_modules/{name}/package.json"),
        &format!(r#"{{"name":"{name}","main":"{main}"}}"#),
    );
    p.file(&format!("node_modules/{name}/{main}"), src);
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

#[test]
fn vi_mock_factories_import_actual_do_mock_and_unmock() {
    let p = esbuild_project("rb-mr-mocks");
    p.file(
        "src/dep.ts",
        "export const a = 1;\nexport function f() { return 'real'; }\nexport function g() { return 'g-real'; }\n",
    );
    p.file("src/other.ts", "export const o = 'real-o';\n");
    p.file("src/sync.ts", "export const s = 'real-s';\n");
    p.file("src/fresh.ts", "export const fr = 'fresh';\n");
    p.file(
        "src/auto.ts",
        "export const prim = 3;\nexport function fn() { return 'auto-real'; }\n",
    );
    p.file(
        "src/useDep.ts",
        "import { f } from './dep';\nexport const callF = () => f();\n",
    );
    p.file("src/thenable.ts", "export const t = 'real-t';\n");
    p.file(
        "mocks.test.ts",
        r#"import { a, f, g } from './src/dep';
import { o } from './src/other';
import { s } from './src/sync';
import { prim, fn } from './src/auto';
import { callF } from './src/useDep';
import { t } from './src/thenable';
vi.mock('./src/dep', async (importOriginal) => {
  const real = await importOriginal();
  return { ...real, f: vi.fn(() => 'mocked') };
});
vi.mock('./src/other', async () => ({ o: 'async-mock' }));
vi.mock('./src/sync', () => ({ s: 'sync-mock' }));
vi.mock('./src/thenable', () => ({ then: (res) => res({ t: 'from-thenable' }) }));
describe('mocks', () => {
  it('partial mock keeps the real exports', () => {
    expect(a).toBe(1); expect(f()).toBe('mocked'); expect(g()).toBe('g-real');
    expect(callF()).toBe('mocked');
  });
  it('async factory', () => { expect(o).toBe('async-mock'); });
  it('sync factory', () => { expect(s).toBe('sync-mock'); });
  it('automock', () => {
    expect(prim).toBe(3); expect(vi.isMockFunction(fn)).toBe(true); expect(fn()).toBe(undefined);
  });
  it('thenable factory result is awaited', () => { expect(t).toBe('from-thenable'); });
  it('importActual: cached real, mocked-never-loaded, unmocked, builtin, unresolvable', async () => {
    expect((await vi.importActual('./src/dep')).f()).toBe('real');
    expect((await vi.importActual('./src/sync')).s).toBe('real-s');
    expect(require('./src/sync').s).toBe('sync-mock');
    expect((await vi.importActual('./src/fresh')).fr).toBe('fresh');
    expect(typeof (await vi.importActual('node:path')).join).toBe('function');
    expect(await vi.importActual('./src/nope')).toBe(undefined);
  });
  it('doMock, unmock and resetModules', async () => {
    vi.resetModules();
    vi.doMock('./src/other', () => ({ o: 'do-mock' }));
    expect((await import('./src/other')).o).toBe('do-mock');
    vi.unmock('./src/other');
    vi.unmock('./src/never-existed');
    expect((await import('./src/other')).o).toBe('real-o');
  });
});
// factory-less (automock) form (code may follow it since turbo-test#31 item 4)
vi.mock('./src/auto');
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 7);
}

#[test]
fn reset_modules_reruns_app_modules_only() {
    let p = esbuild_project("rb-mr-reset");
    pkg(
        &p,
        "countpkg",
        "index.js",
        "globalThis.__pkgRuns = (globalThis.__pkgRuns || 0) + 1; module.exports = { n: globalThis.__pkgRuns };\n",
    );
    p.file(
        "src/counter.cjs",
        "globalThis.__appRuns = (globalThis.__appRuns || 0) + 1; exports.n = globalThis.__appRuns;\n",
    );
    p.file("src/mocked.cjs", "exports.v = 'real';\n");
    p.file(
        "reset.test.ts",
        r#"vi.mock('./src/mocked.cjs', () => ({ v: 'mock' }));
test('resetModules', () => {
  expect(require('./src/counter.cjs').n).toBe(1);
  expect(require('./src/counter.cjs').n).toBe(1);
  expect(require('countpkg').n).toBe(1);
  expect(require('./src/mocked.cjs').v).toBe('mock');
  vi.resetModules();
  expect(require('./src/counter.cjs').n).toBe(2);
  expect(require('countpkg').n).toBe(1);
  expect(require('./src/mocked.cjs').v).toBe('mock');
  expect(typeof require('node:fs').existsSync).toBe('function');
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}

#[test]
fn native_require_resolves_every_module_kind() {
    let p = esbuild_project("rb-mr-require");
    pkg(
        &p,
        "fakepkg",
        "index.js",
        "module.exports = { pkg: 'fake' };\n",
    );
    pkg(&p, "badpkg", "index.js", "throw new Error('pkg boom');\n");
    p.file("src/other.ts", "export const o = 'real-o';\n");
    p.file("src/j.json", r#"{"j": [1, 2]}"#);
    p.file("src/st.css", ".x{}\n");
    p.file("src/addon.node", "not a dylib\n");
    p.file(
        "src/cyc-a.cjs",
        "exports.name = 'a';\nconst b = require('./cyc-b.cjs');\nexports.fromB = b.name + ':' + b.sawA;\n",
    );
    p.file(
        "src/cyc-b.cjs",
        "const a = require('./cyc-a.cjs');\nexports.name = 'b';\nexports.sawA = a.name;\n",
    );
    p.file(
        "src/shape.cjs",
        "module.exports = function hello() { return 'hi'; };\nmodule.exports.extra = 3;\n",
    );
    p.file(
        "src/withdefault.cjs",
        "exports.default = 'dflt'; exports.named = 'n';\n",
    );
    p.file(
        "natives.test.ts",
        r#"import def, { named } from './src/withdefault.cjs';
const here = globalThis.__ttDir;
describe('native require', () => {
  it('builtins + test api', () => {
    expect(typeof require('fs').readFileSync).toBe('function');
    expect(typeof require('node:path').join).toBe('function');
    expect(typeof require('vitest').vi.fn).toBe('function');
    expect(typeof require('@jest/globals').jest.fn).toBe('function');
  });
  it('json + css', () => {
    expect(require('./src/j.json').j).toEqual([1, 2]);
    expect(require('./src/j.json')).toBe(require('./src/j.json'));
    expect(require('./src/st.css')).toBeDefined();
  });
  it('packages', () => {
    expect(require('fakepkg').pkg).toBe('fake');
    expect(() => require('badpkg')).toThrow('pkg boom');
  });
  it('a broken native addon throws', () => {
    expect(() => require('./src/addon.node')).toThrow(/^native addon .*addon\.node: /);
  });
  it('cycles return the live partial exports', () => {
    expect(require('./src/cyc-a.cjs').fromB).toBe('b:a');
  });
  it('module.exports shapes', () => {
    const h = require('./src/shape.cjs');
    expect(h()).toBe('hi'); expect(h.extra).toBe(3);
    expect(def).toBe('dflt'); expect(named).toBe('n');
  });
  it('unresolvable', () => { expect(() => require('./src/nope')).toThrow("Cannot find module './src/nope'"); });
  it('raw natives: default ESM conditions, ignored registrations', () => {
    expect(globalThis.__nativeRequire(here, './src/other').o).toBe('real-o');
    globalThis.__ttRegisterMock(here, './src/other', undefined);
    globalThis.__ttRegisterMock(here, './src/does-not-exist', { x: 1 });
    expect(globalThis.__nativeRequire(here, './src/other').o).toBe('real-o');
  });
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 8);
}

#[test]
fn cjs_loader_diagnostics_for_unreadable_unparsable_and_throwing_modules() {
    let p = esbuild_project("rb-mr-cjsdiag");
    std::fs::write(p.path("badutf.cjs"), b"exports.x = \"\xff\xfe\";\n").unwrap();
    p.file("syntax.cjs", "exports.a = ;\n");
    p.file("throws.cjs", "throw new Error('body boom');\n");
    // valid to the transforms, rejected by V8: the native output fails to compile and so does
    // the esbuild retry
    p.file("rx.ts", "export const r = /(?<a>x)(?<a>y)/;\n");
    p.file(
        "diag.test.ts",
        r#"test('load failures', () => {
  expect(() => require('./badutf.cjs')).toThrow('failed to load ./badutf.cjs');
  expect(() => require('./syntax.cjs')).toThrow("Unexpected token ';'");
  expect(() => require('./throws.cjs')).toThrow('body boom');
  expect(() => require('./rx')).toThrow('Invalid regular expression');
});
"#,
    );
    // esbuild-only transform: the retry would produce the same code, so it is skipped
    let esb = turbo_test(&p.dir, &[], &[("TURBO_NATIVE_CJS", "0")]);
    assert_passes(&esb, 1);
    // the loader's diagnostics stay off stderr (vitest prints none, turbo-test#113)
    for r in [&esb, &turbo_test(&p.dir, &[], &[])] {
        assert_passes(r, 1);
        for noise in [
            "cjs read/transform failed",
            "cjs compile failed",
            "cjs body threw",
        ] {
            assert!(!r.stderr.contains(noise), "{noise}\n{}", r.all());
        }
    }
    let dbg = turbo_test(&p.dir, &[], &[("TURBO_CJS_DEBUG", "1")]);
    assert_passes(&dbg, 1);
    let line = dbg
        .stderr
        .lines()
        .find(|l| l.contains("cjs body threw: "))
        .unwrap_or_else(|| panic!("{}", dbg.all()));
    assert!(
        line.contains("throws.cjs :: Error: body boom :: Error: body boom | "),
        "{line}"
    );
    // TURBO_CJS_DEBUG prints them
    let e = &dbg.stderr;
    assert!(
        e.contains("  cjs read/transform failed: ") && e.contains("badutf.cjs"),
        "{e}"
    );
    assert!(
        e.contains("  cjs compile failed: ") && e.contains("syntax.cjs :: SyntaxError"),
        "{e}"
    );
    assert!(
        e.contains("rx.ts :: SyntaxError: Invalid regular expression"),
        "{e}"
    );
}

#[test]
fn decorator_file_that_throws_is_retried_once_with_metadata() {
    let p = esbuild_project("rb-mr-deco");
    p.file(
        "tsconfig.json",
        r#"{"compilerOptions":{"experimentalDecorators":true,"emitDecoratorMetadata":true}}"#,
    );
    p.file(
        "deco.ts",
        "function dec(_t: any) {}\n@dec\nexport class A {}\n\
         globalThis.__decoLoads = (globalThis.__decoLoads || 0) + 1;\nthrow new Error('deco boom');\n",
    );
    p.file(
        "plain.ts",
        "globalThis.__plainLoads = (globalThis.__plainLoads || 0) + 1;\nthrow new Error('plain boom');\n",
    );
    p.file(
        "d.test.ts",
        r#"test('retry', () => {
  expect(() => require('./deco')).toThrow('deco boom');
  expect(globalThis.__decoLoads).toBe(2);
  expect(() => require('./plain')).toThrow('plain boom');
  expect(globalThis.__plainLoads).toBe(1);
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}

#[test]
fn e2e_entries_stub_broken_node_modules_unless_disabled() {
    let p = esbuild_project("rb-mr-e2e");
    pkg(&p, "badpkg", "index.js", "throw new Error('pkg boom');\n");
    pkg(
        &p,
        "needsmissing",
        "index.js",
        "const m = require('./nope-sibling'); const m2 = require('./nope-sibling');\n\
         module.exports = { got: typeof m, same: m === m2, call: typeof m.anything() };\n",
    );
    p.file("src/addon.node", "not a dylib\n");
    p.file("src/throws.cjs", "throw new Error('body boom');\n");
    p.file(
        "e2e/stub.test.ts",
        r#"describe('e2e lenient', () => {
  it('stubs a failing package', () => {
    const b = require('badpkg');
    expect(typeof b).toBe('function');
    expect(typeof b.anything.deeper).toBe('function');
    expect(typeof new b()).toBe('function');
    expect(typeof b()).toBe('function');
    expect(typeof b.default).toBe('function');
    expect(b.__esModule).toBe(true);
    expect('x' in b).toBe(true);
    b.y = 1;
  });
  it('stubs unresolvable requires from node_modules', () => {
    const n = require('needsmissing');
    expect(n.got).toBe('function'); expect(n.same).toBe(true); expect(n.call).toBe('function');
  });
  it('stubs an addon', () => { expect(typeof require('../src/addon.node')).toBe('function'); });
  it('app modules still fail', () => {
    expect(() => require('../src/throws.cjs')).toThrow('body boom');
  });
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 4);
    let strict = turbo_test(&p.dir, &[], &[("TURBO_NO_STUB_FAIL", "1")]);
    assert_eq!(strict.code, Some(1), "{}", strict.all());
    for msg in [
        "stubs a failing package: pkg boom",
        "stubs unresolvable requires from node_modules: Cannot find module './nope-sibling'",
        "stubs an addon: native addon ",
    ] {
        assert!(strict.stdout.contains(msg), "{msg}\n{}", strict.all());
    }
}

#[test]
fn lazy_stub_barrels_never_load_the_real_package() {
    let p = esbuild_project("rb-mr-lazy");
    // A tiny react stand-in: the stub factory only needs forwardRef + createElement.
    pkg(
        &p,
        "react",
        "index.js",
        "exports.forwardRef = (render) => { const C = (props) => render(props, 'REF'); return C; };\n\
         exports.createElement = (type, props) => ({ type, props });\n",
    );
    // the real barrel would throw if it were ever evaluated
    pkg(
        &p,
        "fake-icons",
        "index.js",
        "throw new Error('real barrel loaded');\n",
    );
    p.file(
        "lazy.test.ts",
        r#"import { AddIcon, Close } from 'fake-icons';
import Star from 'fake-icons/Star.js';
import Deep from 'fake-icons/esm/Deep';
test('lazy stubs', () => {
  expect(AddIcon.displayName).toBe('AddIcon');
  expect(AddIcon.muiName).toBe('SvgIcon');
  const el = AddIcon({ className: 'big', sx: 1, title: 't' });
  expect(el.type).toBe('svg');
  expect(el.props['data-testid']).toBe('AddIconIcon');
  expect(el.props['data-icon']).toBe('AddIcon');
  expect(el.props.className).toBe('MuiSvgIcon-root big');
  expect(el.props.title).toBe('t');
  expect(el.props.sx).toBe(undefined);
  expect(el.props.ref).toBe('REF');
  expect(Close({ 'data-testid': 'x' }).props['data-testid']).toBe('x');
  expect(Close().props.className).toBe('MuiSvgIcon-root');
  expect(Star.displayName).toBe('Star');
  expect(Deep.displayName).toBe('Deep');
  const ns = require('fake-icons');
  expect(ns).toBe(require('fake-icons'));
  expect(ns.__turboLazyStub).toBe(true);
  expect(ns.__esModule).toBe(false);
  expect(ns.default.displayName).toBe('default');
  expect(ns[Symbol.iterator]).toBe(undefined);
  expect('Anything' in ns).toBe(true);
  expect(Object.keys(ns)).toEqual([]);
  ns.Custom = 5; expect(ns.Custom).toBe(5);
  delete ns.Custom;
  Object.defineProperty(ns, 'Z', { value: 1 });
});
"#,
    );
    let r = turbo_test(
        &p.dir,
        &[],
        &[("TURBO_LAZY_STUBS", " other-barrel, ,fake-icons ")],
    );
    assert_passes(&r, 1);
    // Without the env list the real barrel is evaluated (and throws).
    let real = turbo_test(&p.dir, &[], &[("TURBO_CJS_DEBUG", "1")]);
    assert_ne!(real.code, Some(0), "{}", real.all());
    assert!(real.all().contains("real barrel loaded"), "{}", real.all());
}

#[test]
fn lazy_stub_without_react_falls_back_to_the_real_package() {
    let p = esbuild_project("rb-mr-lazy-noreact");
    pkg(
        &p,
        "fake-icons",
        "index.js",
        "exports.Real = 'real-export';\n",
    );
    p.file(
        "nolazy.test.ts",
        "test('fallback', () => { expect(require('fake-icons').Real).toBe('real-export'); });\n",
    );
    let r = turbo_test(&p.dir, &[], &[("TURBO_LAZY_STUBS", "fake-icons")]);
    assert_passes(&r, 1);
}

/// Compile a trivial Node-API addon (module init returns `exports` untouched). None when no C
/// compiler is available.
fn build_addon(p: &Project, rel: &str) -> Option<()> {
    let c = p.file(
        "addon_src/ok.c",
        "typedef void* napi_value; typedef void* napi_env;\n\
         napi_value napi_register_module_v1(napi_env env, napi_value exports) { (void)env; return exports; }\n",
    );
    let out = p.path(rel);
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut cmd = std::process::Command::new("cc");
    cmd.arg("-shared");
    if cfg!(target_os = "macos") {
        cmd.args(["-undefined", "dynamic_lookup"]);
    } else {
        cmd.arg("-fPIC");
    }
    let ok = cmd
        .arg("-o")
        .arg(&out)
        .arg(&c)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(())
}

#[test]
fn native_addon_loads_once_and_is_cached() {
    let p = esbuild_project("rb-mr-addon");
    if build_addon(&p, "src/ok.node").is_none() {
        eprintln!("skipping: no C compiler");
        return;
    }
    p.file(
        "addon.test.ts",
        r#"test('addon', () => {
  const a = require('./src/ok.node');
  expect(typeof a).toBe('object');
  expect(require('./src/ok.node')).toBe(a);
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_passes(&r, 1);
}

#[test]
fn require_of_esm_from_cjs_without_the_module_runner() {
    // TURBO_NO_MR: a CJS module's require() of an ES module loads + links + evaluates it and
    // returns the namespace (Node 22 semantics); failures surface as distinct errors.
    let p = Project::new("rb-nomr-esm");
    p.file("package.json", r#"{"name":"rb-nomr-esm"}"#);
    p.file("x.mjs", "export const m = 'esm-val'; export default 'd';\n");
    p.file("broken.mjs", "export const = ;\n");
    p.file(
        "badlink.mjs",
        "import { nope } from './x.mjs'; export const y = nope;\n",
    );
    p.file("bridge.cjs", "exports.load = (s) => require(s);\n");
    p.file("node_modules/esmpkg/index.mjs", "export const = ;\n");
    p.file(
        "node_modules/esmpkg/package.json",
        r#"{"name":"esmpkg","main":"index.mjs"}"#,
    );
    let body = r#"
test('cjs requiring esm', () => {
  const ns = bridge.load('./x.mjs');
  expect(ns.m).toBe('esm-val'); expect(ns.default).toBe('d');
  expect(bridge.load('./x.mjs')).toBe(ns);
  expect(() => bridge.load('./broken.mjs')).toThrow('failed to load ESM ./broken.mjs');
  expect(() => bridge.load('./badlink.mjs')).toThrow('failed to instantiate ESM ./badlink.mjs');
  PKG
});
"#;
    p.file(
        "r.test.js",
        &format!(
            "import bridge from './bridge.cjs';{}",
            body.replace(
                "PKG",
                "expect(() => bridge.load('esmpkg')).toThrow('failed to load ESM esmpkg');"
            )
        ),
    );
    p.file(
        "e2e/r.test.js",
        &format!(
            "import bridge from '../bridge.cjs';{}",
            body.replace(
                "PKG",
                "expect(typeof bridge.load('esmpkg')).toBe('function');"
            )
        ),
    );
    let r = turbo_test(&p.dir, &[], &[("TURBO_NO_MR", "1")]);
    assert_passes(&r, 2);
}

#[test]
fn an_exception_escaping_the_entry_load_is_the_load_error() {
    let p = esbuild_project("rb-mr-entry-exc");
    p.file(
        "g.test.ts",
        "test('t', () => {});\n\
         Object.defineProperty(module, 'exports', { get() { throw new Error('exports getter boom'); } });\n",
    );
    for env in [&[][..], &[("TURBO_REUSE_ISOLATE", "1")][..]] {
        let r = turbo_test(&p.dir, &[], env);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert!(
            r.stdout.lines().any(|l| l.starts_with("ERROR")
                && l.ends_with("g.test.ts  (Error: exports getter boom)")),
            "{env:?}\n{}",
            r.all()
        );
    }
}

#[test]
fn setup_file_mocks_reach_test_files_with_vitest_missing_export_semantics() {
    let p = esbuild_project("rb-mr-setup-mocks");
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./setup.ts'] } };\n",
    );
    p.file(
        "src/svc.ts",
        "export const a = 'real-a';\nexport const b = 'real-b';\n",
    );
    p.file(
        "setup.ts",
        "vi.mock('./src/svc', () => { return { a: 'setup-a' }; });\n\
         vi.mock('*.svg?url', () => ({ default: 'x' }));\n",
    );
    // `b` is imported but missing from the factory: the mock module still links; reading `b`
    // throws, as in vitest: on the module runner a property access, on the ESM graph a read
    // through the namespace (esm_spy.rs reads imports through it, turbo-test#146)
    p.file(
        "s.test.ts",
        "import { a, b } from './src/svc';\n\
         test('setup mock', () => { expect(a).toBe('setup-a'); \
         expect(() => b).toThrow('No \"b\" export is defined on the \"./src/svc\" mock'); });\n",
    );
    p.file(
        "legacy.test.ts",
        "import { a, b } from './src/svc';\n\
         test('setup mock', () => { expect(a).toBe('setup-a'); \
         expect(() => b).toThrow('No \"b\" export is defined on the \"./src/svc\" mock'); });\n",
    );
    assert_passes(&turbo_test(&p.dir, &["s.test.ts"], &[]), 1);
    assert_passes(
        &turbo_test(&p.dir, &["legacy.test.ts"], &[("TURBO_NO_MR", "1")]),
        1,
    );
}
