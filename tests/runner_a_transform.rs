//! Transform routing through the real CLI: emitDecoratorMetadata files via the project's
//! TypeScript (module-runner + ESM-graph paths) with the oxc fallback, a broken/empty/invalid
//! TypeScript, the native (oxc) app + dependency transforms with their esbuild fallbacks and
//! strict mode, `--coverage` (esbuild inline maps), esbuild transform failures, per-file
//! environment pragmas, and the warm transform cache.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// (passed, failed, per-file statuses) from a `--reporter json` run.
fn counts(r: &common::Run) -> (u64, u64, Vec<String>) {
    let j = json_report(r);
    let statuses = j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(common::file_status)
        .collect();
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
        statuses,
    )
}

fn passed(n: u64, files: usize) -> (u64, u64, Vec<String>) {
    (n, 0, vec!["passed".to_string(); files])
}

/// Cache entries whose file name starts with `prefix`.
fn cached(p: &Project, prefix: &str) -> usize {
    std::fs::read_dir(p.path(".tt-cache"))
        .map(|d| {
            d.filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(prefix)
            })
            .count()
        })
        .unwrap_or(0)
}

const REFLECT_SETUP: &str = r#"(function () {
  const store = new WeakMap();
  const k = (mk, p) => String(mk) + ' ' + String(p);
  const slot = (t) => { let m = store.get(t); if (!m) { m = new Map(); store.set(t, m); } return m; };
  Reflect.defineMetadata = (mk, mv, t, p) => { slot(t).set(k(mk, p), mv); };
  Reflect.getMetadata = (mk, t, p) => { const m = store.get(t); return m ? m.get(k(mk, p)) : undefined; };
  Reflect.metadata = (mk, mv) => (t, p) => Reflect.defineMetadata(mk, mv, t, p);
})();
"#;

/// A decorator project: a model whose field decorator THROWS without `design:type` metadata,
/// one field typed as an imported interface (type-only import).
fn decorator_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", r#"{ "name": "deco" }"#);
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "experimentalDecorators": true, "emitDecoratorMetadata": true } }"#,
    );
    p.file("setup.ts", REFLECT_SETUP);
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./setup.ts'] } };\n",
    );
    p.file("iface.ts", "export interface Shape { a: number }\n");
    p.file(
        "model.ts",
        r#"import { Shape } from './iface';
function RequireType(target: any, key: string) {
  const t = Reflect.getMetadata('design:type', target, key);
  if (typeof t !== 'function') throw new Error('no design:type for ' + key);
  (target.constructor.__fieldTypes || (target.constructor.__fieldTypes = {}))[key] = t.name;
}
export class Model {
  @RequireType count!: number;
  @RequireType label!: string;
  @RequireType shape!: Shape;
}
"#,
    );
    p.file(
        "a.test.ts",
        "import { Model } from './model';\ntest('design:type metadata', () => expect((Model as any).__fieldTypes).toEqual({ count: 'Number', label: 'String', shape: 'Object' }));\n",
    );
    p
}

/// More import shapes for the metadata of decorated members: an aliased interface, a class, a
/// default-imported interface and a value import sharing their declaration, a qualified type,
/// `implements` and `typeof` positions, plus constructor parameter types. `k.test.ts` checks every `design:*` type.
fn add_metadata_import_kinds(p: &Project) {
    p.file(
        "kinds.ts",
        r#"export interface Opts { o: 1 }
export class Thing {}
export const VALUE = 3;
export default interface Def { d: 1 }
export namespace Other { export interface Inner { i: 1 } }
"#,
    );
    p.file(
        "model2.ts",
        r#"import Def, { Opts as O, Thing, VALUE, Other } from './kinds';
function Field(target: any, key: string) {
  (target.constructor.__types || (target.constructor.__types = {}))[key] =
    Reflect.getMetadata('design:type', target, key).name;
}
function Param(_t: any, _k: any, _i: number) {}
type Value = typeof VALUE;
class Impl implements O { o!: 1; }
export class M2 {
  @Field opts!: O;
  @Field thing!: Thing;
  @Field def!: Def;
  @Field inner!: Other.Inner;
  value: Value = VALUE;
  constructor(@Param a: O, @Param t: Thing) {}
}
"#,
    );
    p.file(
        "k.test.ts",
        r#"import { M2 } from './model2';
test('design:type / design:paramtypes for each import shape', () => {
  expect((M2 as any).__types).toEqual({ opts: 'Object', thing: 'Thing', def: 'Object', inner: 'Object' });
  expect(Reflect.getMetadata('design:paramtypes', M2, undefined).map((t: any) => t.name)).toEqual(['Object', 'Thing']);
  expect(new M2(null as any, null as any).value).toBe(3);
});
"#,
    );
}

#[test]
fn decorator_metadata_via_typescript_on_the_module_runner() {
    let p = decorator_project("ta-deco-mr-tsc");
    p.with_esbuild();
    p.with_typescript();
    add_metadata_import_kinds(&p);
    // a decorated test file whose vi.mock factory shares a module `let` with the tests
    p.file("svc.ts", "export const svc = () => 'real';\n");
    p.file(
        "b.test.ts",
        r#"import { svc } from './svc';
let capturedValue;
vi.mock('./svc', () => { capturedValue = 'from-factory'; return { svc: () => 'mocked' }; });
function Tag(target: any) { target.tagged = true; }
@Tag
class Local {}
test('decorated entry with a hoisted mock', () => {
  expect(svc()).toBe('mocked');
  expect(capturedValue).toBe('from-factory');
  expect((Local as any).tagged).toBe(true);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(3, 3), "{}", r.all());
}

#[test]
fn decorator_metadata_falls_back_to_oxc_without_typescript() {
    let p = decorator_project("ta-deco-mr-oxc");
    p.with_esbuild();
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(1, 1), "{}", r.all());
}

#[test]
fn decorator_metadata_on_the_esm_graph_path() {
    // no esbuild: the entry + model load as ESM; typescript (tsc_root) lowers the decorators.
    // A plain `import { Shape }` of an interface links: the type-only binding is read through a
    // namespace import, so its `design:type` is `Object` (as with `import type`).
    let p = decorator_project("ta-deco-esm-tsc");
    p.with_typescript();
    add_metadata_import_kinds(&p);
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(2, 2), "{}", r.all());
    // neither esbuild nor typescript: oxc legacy-decorator lowering (no type information, so
    // the interface needs `import type` there)
    let model = std::fs::read_to_string(p.path("model.ts")).unwrap();
    let q = decorator_project("ta-deco-esm-oxc");
    q.file(
        "model.ts",
        &model.replace("import { Shape }", "import type { Shape }"),
    );
    q.file(
        "a.test.ts",
        "import { Model } from './model';\ntest('decorators run', () => expect(Object.keys((Model as any).__fieldTypes)).toEqual(['count', 'label', 'shape']));\n",
    );
    let r = turbo_test(&q.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(1, 1), "{}", r.all());
}

/// A fake `typescript` package whose transpileModule misbehaves.
fn fake_typescript(p: &Project, body: &str) {
    p.file(
        "node_modules/typescript/package.json",
        r#"{ "name": "typescript", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/typescript/index.js",
        &format!(
            "exports.ModuleKind = {{ ESNext: 99 }};\nexports.ScriptTarget = {{ ES2021: 8 }};\nexports.JsxEmit = {{ ReactJSX: 4 }};\nexports.transpileModule = () => {{ {body} }};\n"
        ),
    );
}

#[test]
fn a_misbehaving_typescript_falls_back_to_other_transforms() {
    // A decorator that does not need metadata, so every fallback transform can load it.
    let mk = |name: &str, esbuild: bool, ts_body: &str| {
        let p = Project::new(name);
        if esbuild {
            p.with_esbuild();
        }
        fake_typescript(&p, ts_body);
        p.file("package.json", r#"{ "name": "fake-ts" }"#);
        p.file(
            "tsconfig.json",
            r#"{ "compilerOptions": { "experimentalDecorators": true, "emitDecoratorMetadata": true } }"#,
        );
        p.file(
            "model.ts",
            "function Tag(target: any) { target.tagged = true; }\n@Tag\nexport class M {}\n",
        );
        p.file(
            "a.test.ts",
            "import { M } from './model';\ntest('decorated', () => expect((M as any).tagged).toBe(true));\n",
        );
        p
    };
    for (name, esbuild, body) in [
        // node exits non-zero
        ("ta-ts-throws-esm", false, "throw new Error('boom');"),
        // empty output
        ("ta-ts-empty-esm", false, "return { outputText: '   ' };"),
        ("ta-ts-throws-mr", true, "throw new Error('boom');"),
        // output esbuild cannot convert to CJS
        (
            "ta-ts-invalid-mr",
            true,
            "return { outputText: 'export const = ;' };",
        ),
    ] {
        let p = mk(name, esbuild, body);
        let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
        assert_eq!(counts(&r), passed(1, 1), "{name}\n{}", r.all());
    }
}

#[test]
fn native_transform_strict_mode_and_esbuild_fallbacks() {
    let p = Project::new("ta-native");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "native" }"#);
    // Top-level `return` is valid CommonJS but not an ES module: the native app emitter and
    // the native dep bundler both reject it, esbuild accepts it.
    p.file(
        "helper.js",
        "module.exports = { v: 7 };\nif (module.exports) return;\n",
    );
    p.file("node_modules/odd/package.json", r#"{ "name": "odd" }"#);
    p.file(
        "node_modules/odd/index.js",
        "module.exports = { o: 8 };\nif (module.exports) return;\nmodule.exports = 0;\n",
    );
    // import.meta in an app module and a dep: the native emit and bundler lower it (runtime.js
    // __ttImportMeta, turbo-test#123), esbuild's transforms fill it in
    p.file("meta.ts", "export const m = typeof import.meta;\n");
    p.file(
        "node_modules/metadep/package.json",
        r#"{ "name": "metadep", "main": "index.mjs" }"#,
    );
    p.file(
        "node_modules/metadep/index.mjs",
        "export const md = typeof import.meta;\n",
    );
    p.file(
        "a.test.ts",
        r#"import { m } from './meta';
import { md } from 'metadep';
test('app CJS with top-level return', () => expect(require('./helper.js').v).toBe(7));
test('dep CJS with top-level return', () => expect(require('odd').o).toBe(8));
test('import.meta in app + dep', () => { expect(m).toBe('object'); expect(md).toBe('object'); });
test('a dep no transform can load', () => expect(() => require('broken')).toThrow("Unexpected token ';'"));
"#,
    );
    p.file(
        "node_modules/broken/package.json",
        r#"{ "name": "broken" }"#,
    );
    p.file("node_modules/broken/index.js", "module.exports = ;\n");
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(4, 1), "{}", r.all());
    assert!(cached(&p, "mrdep-") >= 1, "esbuild dep bundle cached");
    assert!(cached(&p, "ntv-") >= 1, "native app transform cached");
    assert!(cached(&p, "ntvdep-") >= 1, "native dep bundle cached");

    // strict: no esbuild fallback for native failures — each surfaces as a read/transform error
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NATIVE_CJS_STRICT", "1"), ("TURBO_CJS_DEBUG", "1")],
    );
    for f in ["helper.js", "node_modules/odd/index.js"] {
        assert!(
            r.stderr.contains(&format!(
                "cjs read/transform failed: {}",
                p.path(f).display()
            )),
            "{f}\n{}",
            r.all()
        );
    }

    // native transforms off: esbuild for app files and dep bundles, same results
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")],
    );
    assert_eq!(counts(&r), passed(4, 1), "{}", r.all());
}

#[test]
fn coverage_routes_app_and_dep_transforms_through_esbuild() {
    let p = Project::new("ta-coverage");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "cov" }"#);
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "experimentalDecorators": true, "emitDecoratorMetadata": true } }"#,
    );
    p.file(
        "src/math.ts",
        "function Tag(t: any) { t.tagged = true; }\n@Tag\nexport class Calc {}\nexport function add(a: number, b: number) {\n  return a + b;\n}\nexport function unused() {\n  return 0;\n}\n",
    );
    p.file("node_modules/dep/package.json", r#"{ "name": "dep" }"#);
    p.file("node_modules/dep/index.js", "exports.one = 1;\n");
    p.file(
        "src/math.test.ts",
        "import { add, Calc } from './math';\nimport { one } from 'dep';\ntest('add', () => { expect(add(one, 2)).toBe(3); expect((Calc as any).tagged).toBe(true); });\n",
    );
    let r = turbo_test(
        &p.dir,
        &[
            "--coverage",
            "--coverage-reporter",
            "json-summary",
            "--coverage-dir",
            "cov",
        ],
        &[],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    let summary = std::fs::read_to_string(p.path("cov/coverage-summary.json")).unwrap();
    let j: serde_json::Value = serde_json::from_str(&summary).unwrap();
    let file = j
        .as_object()
        .unwrap()
        .iter()
        .find(|(k, _)| k.ends_with("src/math.ts"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("math.ts missing from {summary}"));
    let pct = file["lines"]["pct"].as_f64().unwrap();
    assert!(pct > 0.0 && pct < 100.0, "{summary}");
    assert_eq!(
        cached(&p, "ntv-"),
        0,
        "coverage never uses the native emitter"
    );
}

#[test]
fn an_esbuild_syntax_error_is_a_load_error_and_cache_warms() {
    let p = Project::new("ta-esbuild-fail");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "fail" }"#);
    p.file("bad.test.ts", "const x: number = ;\n");
    p.file("ok.test.ts", "test('ok', () => expect(1).toBe(1));\n");
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        counts(&r),
        (1, 0, vec!["error".into(), "passed".into()]),
        "{}",
        r.all()
    );
    assert!(r.stderr.contains("1 load-errors"), "{}", r.all());
    let ok = Project::new("ta-cache-warm");
    ok.with_esbuild();
    ok.file("package.json", r#"{ "name": "warm" }"#);
    ok.file("util.ts", "export const u = 2;\n");
    ok.file(
        "a.test.ts",
        "import { u } from './util';\ntest('u', () => expect(u).toBe(2));\n",
    );
    let cold = turbo_test(&ok.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&cold), passed(1, 1), "{}", cold.all());
    assert!(cold.stderr.contains("cache 0% hit"), "{}", cold.all());
    let warm = turbo_test(&ok.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&warm), passed(1, 1), "{}", warm.all());
    assert!(warm.stderr.contains("cache 100% hit"), "{}", warm.all());
}

#[test]
fn environment_pragmas_and_the_environment_flag() {
    let p = Project::new("ta-env");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "env" }"#);
    p.file(
        "node.test.tsx",
        "// @vitest-environment node\ntest('node env on a .tsx file', () => expect(typeof globalThis['docu' + 'ment']).toBe('undefined'));\n",
    );
    p.file(
        "dom.test.ts",
        "/** @vitest-environment jsdom */\ntest('jsdom env', () => expect(typeof globalThis['docu' + 'ment']).toBe('object'));\n",
    );
    p.file(
        "auto.test.ts",
        "test('auto env: no DOM for plain logic', () => expect(typeof globalThis['docu' + 'ment']).toBe(globalThis.EXPECT_DOM || 'undefined'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), passed(3, 3), "{}", r.all());
    // --environment happy-dom forces a DOM for files without a pragma (the node pragma still wins)
    p.file(
        "auto.test.ts",
        "test('forced env', () => expect(typeof globalThis['docu' + 'ment']).toBe('object'));\n",
    );
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json", "--environment", "happy-dom"],
        &[],
    );
    assert_eq!(counts(&r), passed(3, 3), "{}", r.all());
}

#[test]
fn import_meta_glob_in_jsx_and_js_modules_on_the_esbuild_path() {
    // the glob-expanded module goes to esbuild on stdin with its own loader: `jsx` for a .jsx and
    // a .js module (vitest 4.1.12: both list the sibling module)
    let p = Project::new("ra-glob-jsx");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"ra-glob-jsx","type":"module"}"#);
    p.file("src/mods/a.ts", "export const n = 1;\n");
    p.file(
        "src/v.jsx",
        "export const viewGlob = () => Object.keys(import.meta.glob('./mods/*.ts'));\n",
    );
    p.file(
        "src/p.js",
        "export const plainGlob = () => Object.keys(import.meta.glob('./mods/*.ts', { eager: true }));\n",
    );
    p.file(
        "a.test.ts",
        "import { expect, it } from 'vitest';\nimport { viewGlob } from './src/v.jsx';\nimport { plainGlob } from './src/p.js';\n\
         it('globs', () => { expect([viewGlob(), plainGlob()]).toEqual([['./mods/a.ts'], ['./mods/a.ts']]); });\n",
    );
    for env in [
        &[][..],
        &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")][..],
    ] {
        let r = turbo_test(&p.dir, &["--reporter", "json"], env);
        assert_eq!(counts(&r), passed(1, 1), "{env:?}\n{}", r.all());
    }
}

#[test]
fn the_loader_debug_switch_names_a_module_that_does_not_transform() {
    // TURBO_CJS_DEBUG: the oxc transform error of an app module on the ES-module graph is printed
    // with the module's path; the file fails as without it
    let p = Project::new("ra-xform-debug");
    p.file(
        "package.json",
        r#"{"name":"ra-xform-debug","type":"module"}"#,
    );
    let dep = p.file(
        "dep.ts",
        "export const a = <T,>(x: T) => x;\nexport const b: = 1;\n",
    );
    p.file(
        "a.test.ts",
        "import { it } from 'vitest';\nimport { a } from './dep';\nit('a', () => { a(1); });\n",
    );
    let quiet = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let debug = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_CJS_DEBUG", "1")]);
    let line = format!("transform {}: ", dep.display());
    assert!(!quiet.stderr.contains(&line), "{}", quiet.all());
    assert!(
        debug.stderr.lines().any(|l| l.starts_with(&line)),
        "{}",
        debug.all()
    );
    assert_eq!(counts(&quiet), counts(&debug));
    assert_eq!(counts(&debug).2, ["error"], "{}", debug.all());
}
