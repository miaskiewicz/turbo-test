//! The native ESM→CJS emitter (app files) and package bundler (node_modules) driven end-to-end
//! through the CLI's module runner, including the `TURBO_NATIVE_CJS` / `TURBO_NATIVE_DEPS`
//! opt-outs and `TURBO_NATIVE_CJS_STRICT` (no esbuild fallback).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project, Run};

/// `file name → status` from a `--reporter json` run.
fn statuses(r: &Run) -> Vec<(String, String)> {
    let j = json_report(r);
    let mut v: Vec<(String, String)> = j["testResults"]
        .as_array()
        .unwrap_or_else(|| panic!("no testResults:\n{}", r.all()))
        .iter()
        .map(|t| {
            let name = t["name"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string();
            (name, common::file_status(t))
        })
        .collect();
    v.sort();
    v
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

/// An esbuild project (so the module runner loads tests as CJS) with a stub `react/jsx-runtime`.
fn mr_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file(
        "node_modules/react/package.json",
        "{\"name\":\"react\",\"main\":\"index.js\"}",
    );
    p.file("node_modules/react/index.js", "module.exports = {};\n");
    p.file(
        "node_modules/react/jsx-runtime.js",
        "exports.jsx = (t) => t; exports.jsxs = exports.jsx; exports.Fragment = 'F';\n",
    );
    p
}

#[test]
fn native_emit_keeps_bindings_live_and_bundles_cyclic_deps() {
    let p = mr_project("xform-native");
    p.file("src/m.js", "export function f() { return 1; }\n");
    p.file(
        "src/user.ts",
        "import { f } from './m.js';\nexport const useIt = (): number => f();\n",
    );
    p.file(
        "node_modules/dep/package.json",
        "{\"name\":\"dep\",\"main\":\"index.js\"}",
    );
    p.file(
        "node_modules/dep/index.js",
        "import { b } from './b.js';\nexport const a = () => 'A' + b();\nexport * from './c.js';\n",
    );
    p.file(
        "node_modules/dep/b.js",
        "import { a } from './index.js';\nexport const b = () => 'B';\nexport const viaA = () => a();\n",
    );
    p.file("node_modules/dep/c.js", "export const c = 'C';\n");
    p.file(
        "native.test.ts",
        "import * as m from './src/m.js';\nimport { useIt } from './src/user';\n\
         import { a, c } from 'dep';\nimport { viaA } from 'dep/b.js';\n\
         test('live binding', () => { vi.spyOn(m, 'f').mockReturnValue(42); expect(useIt()).toBe(42); });\n\
         test('dep bundle', () => { expect(a()).toBe('AB'); expect(c).toBe('C'); expect(viaA()).toBe('AB'); });\n",
    );
    // native (default) and the esbuild baseline must agree
    for env in [
        &[][..],
        &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")][..],
    ] {
        let r = turbo_test(&p.dir, &["--reporter", "json"], env);
        assert_eq!(r.code, Some(0), "{env:?}: {}", r.all());
        let j = json_report(&r);
        assert_eq!(j["numPassedTests"], 2, "{env:?}: {}", r.all());
    }
}

/// Names of the `.tt-cache` files starting with `prefix` (`ntv-` = native app transform,
/// `mr-` = esbuild app transform, which the compile-failure retry also writes).
fn cached(p: &Project, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(p.path(".tt-cache"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    v.sort();
    v
}

/// Run `files` of the project under the native emitter in strict mode with a fresh cache, then
/// under the esbuild baseline; both must pass `passed` tests. Under native, every app file must load from
/// the native emitter's output: the only esbuild app transform (`mr-*.cjs`) may be the runner's
/// probe of the test entry, so a second one means a module's native output failed to compile and
/// was silently re-transformed by esbuild.
fn native_and_esbuild_agree(p: &Project, files: &[&str], passed: u64) {
    for env in [
        &[("TURBO_NATIVE_CJS_STRICT", "1")][..],
        &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")][..],
    ] {
        let _ = std::fs::remove_dir_all(p.path(".tt-cache"));
        let args = [&["--reporter", "json"][..], files].concat();
        let r = turbo_test(&p.dir, &args, env);
        assert_eq!(r.code, Some(0), "{env:?}: {}", r.all());
        let j = json_report(&r);
        assert_eq!(j["numPassedTests"], passed, "{env:?}: {}", r.all());
        if env.len() == 1 {
            assert!(!cached(p, "ntv-").is_empty(), "native emitter used");
            assert_eq!(
                cached(p, "mr-").len(),
                1,
                "only the entry probe, no esbuild retry"
            );
        }
    }
}

#[test]
fn dynamic_import_in_a_module_without_static_imports() {
    // turbo-test#37 item 1: a test file whose only import is `import()` is a script to the
    // emitter, which declared `__toESM` only in the module preamble → "__toESM is not defined".
    let p = mr_project("xform-dynonly");
    p.file("m.js", "export const v = 7;\n");
    p.file(
        "a.test.js",
        "test('dyn', async () => { const m = await import('./m.js'); expect(m.v).toBe(7); });\n",
    );
    native_and_esbuild_agree(&p, &[], 1);
}

#[test]
fn dynamic_import_defers_the_load_and_rejects() {
    // turbo-test#37 item 2: the lowered `import()` required the module synchronously, so a
    // module that throws made `import()` throw instead of returning a rejected promise.
    let p = mr_project("xform-dynreject");
    p.file("t.js", "throw new Error('boom');\n");
    p.file("m.js", "export const v = 7;\n");
    p.file(
        "node_modules/ddep/package.json",
        "{\"name\":\"ddep\",\"main\":\"index.js\"}",
    );
    p.file("node_modules/ddep/n.js", "export const n = 1;\n");
    p.file(
        "node_modules/ddep/index.js",
        "import * as ns from './n.js';\nexport { ns };\nexport const load = () => import('./n.js');\n",
    );
    p.file(
        "a.test.js",
        r#"import { ns, load } from 'ddep';
test('rej', async () => {
  const p = (() => { try { return import('./t.js'); } catch (e) { return 'sync:' + e.message; } })();
  expect(typeof p).toBe('object');
  await expect(p).rejects.toThrow('boom');
});
test('bundled dep import() reads the bundled instance', async () => {
  expect(await load()).toBe(ns);
});
test('doMock before import() is seen', async () => {
  vi.doMock('./m.js', () => ({ v: 'mocked' }));
  expect((await import('./m.js')).v).toBe('mocked');
  vi.doUnmock('./m.js');
});
"#,
    );
    native_and_esbuild_agree(&p, &["a.test.js"], 3);
    // Node / vitest evaluate the specifier at the `import()` call; only the load is deferred.
    // (esbuild's lowering, the TURBO_NATIVE_CJS=0 baseline, evaluates it lazily inside `.then`.)
    p.file(
        "eager.test.js",
        "test('specifier is evaluated at the call', async () => {\n\
         let s = './m.js';\nconst p = import(s);\ns = './t.js';\nexpect((await p).v).toBe(7);\n});\n",
    );
    let r = turbo_test(
        &p.dir,
        &["eager.test.js"],
        &[("TURBO_NATIVE_CJS_STRICT", "1")],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn specifiers_with_quotes_and_backslashes_compile_natively() {
    // turbo-test#37 item 3: `require("{src}")` interpolated the specifier raw, so `./q"x.js`
    // emitted `require("./q"x.js")` (and `\\x` a bad hex escape). The module still loaded only
    // because the runner silently retried the compile failure through esbuild.
    let p = mr_project("xform-specesc");
    p.file(
        "q\"x.js",
        "export const q = 'Q';\nglobalThis.__sideEffect = 'S';\n",
    );
    p.file("b\\x.js", "export default 'B';\nexport const n = 'N';\n");
    p.file(
        "lib.js",
        "import './q\"x.js';\nimport { q } from './q\"x.js';\nimport B, * as ns from './b\\\\x.js';\n\
         export { n as reN } from './b\\\\x.js';\nexport * from './q\"x.js';\nexport * as all from './b\\\\x.js';\n\
         export const got = [q, B, ns.n];\n",
    );
    p.file(
        "a.test.js",
        "import { got, reN, q, all } from './lib.js';\n\
         test('escaped specifiers', () => {\n\
         expect(got).toEqual(['Q', 'B', 'N']); expect(reN).toBe('N'); expect(q).toBe('Q');\n\
         expect(all.n).toBe('N'); expect(globalThis.__sideEffect).toBe('S');\n});\n",
    );
    native_and_esbuild_agree(&p, &[], 1);
}

/// App file + dep package the native path can't lower (JSX namespace names) but esbuild can.
fn namespace_jsx_project() -> Project {
    let p = mr_project("xform-strict");
    p.file("src/ns.jsx", "export const el = <a:b />;\n");
    p.file(
        "node_modules/nsdep/package.json",
        "{\"name\":\"nsdep\",\"main\":\"index.jsx\"}",
    );
    p.file(
        "node_modules/nsdep/index.jsx",
        "export const el = <c:d />;\n",
    );
    p.file(
        "app.test.js",
        "import { el } from './src/ns.jsx';\ntest('app', () => { expect(el).toBe('a:b'); });\n",
    );
    p.file(
        "dep.test.js",
        "import { el } from 'nsdep';\ntest('dep', () => { expect(el).toBe('c:d'); });\n",
    );
    p
}

#[test]
fn native_failures_fall_back_to_esbuild_unless_strict() {
    let p = namespace_jsx_project();
    let app_err = format!(
        "cjs read/transform failed: {}",
        p.path("src/ns.jsx").display()
    );
    let dep_err = format!(
        "cjs read/transform failed: {}",
        p.path("node_modules/nsdep/index.jsx").display()
    );

    // default: native fails, esbuild fallback succeeds — silently
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        statuses(&r),
        vec![pair("app.test.js", "passed"), pair("dep.test.js", "passed")]
    );
    assert!(
        !r.stderr.contains("cjs read/transform failed"),
        "{}",
        r.all()
    );

    // strict: both native failures surface as load errors
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NATIVE_CJS_STRICT", "1")],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        statuses(&r),
        vec![pair("app.test.js", "error"), pair("dep.test.js", "error")]
    );
    assert!(r.stderr.contains(&app_err), "{}", r.all());
    assert!(r.stderr.contains(&dep_err), "{}", r.all());

    // an empty STRICT value is off
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NATIVE_CJS_STRICT", "")],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());

    // strict + app emitter opted out (0 or empty): app goes straight to esbuild, dep still strict
    for off in ["0", ""] {
        let r = turbo_test(
            &p.dir,
            &["--reporter", "json"],
            &[("TURBO_NATIVE_CJS_STRICT", "1"), ("TURBO_NATIVE_CJS", off)],
        );
        assert_eq!(
            statuses(&r),
            vec![pair("app.test.js", "passed"), pair("dep.test.js", "error")],
            "{off:?}"
        );
        assert!(!r.stderr.contains(&app_err), "{}", r.all());
        assert!(r.stderr.contains(&dep_err), "{}", r.all());
    }

    // strict + dep bundler opted out: dep goes straight to esbuild, app still strict
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NATIVE_CJS_STRICT", "1"), ("TURBO_NATIVE_DEPS", "0")],
    );
    assert_eq!(
        statuses(&r),
        vec![pair("app.test.js", "error"), pair("dep.test.js", "passed")]
    );
    assert!(r.stderr.contains(&app_err), "{}", r.all());
    assert!(!r.stderr.contains(&dep_err), "{}", r.all());
}

#[test]
fn non_identifier_string_names_import_and_reexport() {
    // turbo-test#35 item 2: the native emitter wrote `import_s.a-b()` (= `import_s.a - b()`,
    // "b is not defined") for string-named imports/re-exports that aren't identifiers.
    let p = mr_project("xform-strnames");
    let s = "const f = () => 7;\nexport { f as \"a-b\", f as 'it\"s\\\\' };\n";
    p.file("s.js", s);
    p.file("re.js", "export { \"a-b\" as c } from \"./s.js\";\n");
    p.file(
        "star.js",
        "export * as \"n-s\" from \"./s.js\";\nimport { \"a-b\" as f } from \"./s.js\";\n\
         export { f as \"q-r\" };\n",
    );
    p.file(
        "node_modules/sdep/package.json",
        "{\"name\":\"sdep\",\"main\":\"index.js\"}",
    );
    p.file("node_modules/sdep/s.js", s);
    p.file(
        "node_modules/sdep/index.js",
        "import { \"a-b\" as ab } from './s.js';\nexport { ab as \"x-y\" };\n\
         export { \"a-b\" as viaRe } from './s.js';\nexport * as \"ns-x\" from './s.js';\n",
    );
    p.file(
        "a.test.js",
        "import { \"a-b\" as ab, 'it\"s\\\\' as its } from './s.js';\nimport { c } from './re.js';\n\
         import { \"n-s\" as ns, \"q-r\" as qr } from './star.js';\n\
         import { \"x-y\" as xy, viaRe, \"ns-x\" as depNs } from 'sdep';\n\
         test('string-named import', () => { expect(ab()).toBe(7); expect(its()).toBe(7); });\n\
         test('string-named re-export', () => { expect(c()).toBe(7); });\n\
         test('export * as / local re-export', () => { expect(ns['a-b']()).toBe(7); expect(qr()).toBe(7); });\n\
         test('bundled dep', () => { expect(xy()).toBe(7); expect(viaRe()).toBe(7); expect(depNs['a-b']()).toBe(7); });\n",
    );
    // native strict (no esbuild fallback can mask the emitter) and the esbuild baseline agree
    for env in [
        &[("TURBO_NATIVE_CJS_STRICT", "1")][..],
        &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")][..],
    ] {
        let r = turbo_test(&p.dir, &["--reporter", "json"], env);
        assert_eq!(r.code, Some(0), "{env:?}: {}", r.all());
        let j = json_report(&r);
        assert_eq!(j["numPassedTests"], 4, "{env:?}: {}", r.all());
    }
}
