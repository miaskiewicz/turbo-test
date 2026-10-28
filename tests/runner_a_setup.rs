//! Setup files and isolate reuse through the real CLI: vitest `setupFiles` (module-runner: loaded
//! like a test file; otherwise bundled, with their vi.mock specifiers externalized and rewritten to
//! absolute paths), setup-mock snapshot/restore
//! and importer invalidation across files on a reused isolate (`isolate: false`,
//! TURBO_REUSE_ISOLATE, TURBO_NO_REUSE), and setup files that fail to bundle.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// Per-file (file name, status, passed, failed) in report order.
fn files(r: &common::Run) -> Vec<(String, String, u64, u64)> {
    json_report(r)["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"]
                    .as_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string(),
                common::file_status(t),
                common::file_counts(t).0,
                common::file_counts(t).1,
            )
        })
        .collect()
}

fn f(name: &str, status: &str, pass: u64, fail: u64) -> (String, String, u64, u64) {
    (name.to_string(), status.to_string(), pass, fail)
}

/// A project whose setup file mocks `src/analytics`, plus three test files that run in order
/// a -> b -> c on one worker: (a) sees the setup mock through two app importers (a diamond in
/// the import graph), (b) re-mocks analytics, (c) must see the setup mock again.
fn reuse_project(name: &str, config: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "reuse" }"#);
    p.file("vitest.config.ts", config);
    p.file(
        "src/analytics.ts",
        "export const track = () => 'real';\nexport const flag = 'real';\n",
    );
    p.file(
        "src/hook.ts",
        "import { track } from './analytics';\nexport const useTrack = () => track();\n",
    );
    p.file(
        "src/hook2.ts",
        "import { flag } from './analytics';\nexport const getFlag = () => flag;\n",
    );
    // node_modules importer of a mocked package: never evicted by importer invalidation
    p.file(
        "node_modules/fakedep/package.json",
        r#"{ "name": "fakedep" }"#,
    );
    p.file("node_modules/fakedep/index.js", "exports.v = 'real';\n");
    p.file(
        "node_modules/nmuser/package.json",
        r#"{ "name": "nmuser" }"#,
    );
    p.file(
        "node_modules/nmuser/index.js",
        "const d = require('fakedep');\nexports.get = () => d.v;\n",
    );
    p.file(
        "setup.ts",
        r#"import { vi } from 'vitest';
import { track } from './src/analytics';
vi.mock('./src/analytics', () => { return { track: () => 'setup', flag: 'setup' }; });
globalThis.SETUP_TRACK = track;
"#,
    );
    p.file(
        "src/a.test.ts",
        r#"import { useTrack } from './hook';
import { getFlag } from './hook2';
import { get } from 'nmuser';
test('setup mock reaches app importers', () => {
  expect(useTrack()).toBe('setup');
  expect(getFlag()).toBe('setup');
  expect(get()).toBe('real');
});
test('setup file ran and its externalized import resolved', () => {
  expect(typeof globalThis.SETUP_TRACK).toBe('function');
});
"#,
    );
    p.file(
        "src/b.test.ts",
        r#"import { useTrack } from './hook';
import { getFlag } from './hook2';
vi.mock('./analytics', () => { return { track: () => 'fileB', flag: 'B' }; });
vi.mock('fakedep', () => { return { v: 'mocked' }; });
test('a re-mock reaches importers', () => {
  expect(useTrack()).toBe('fileB');
  expect(getFlag()).toBe('B');
});
"#,
    );
    p.file(
        "src/c.test.ts",
        r#"import { useTrack } from './hook';
import { track } from './analytics';
test('the setup mock is restored for the next file', () => {
  expect(track()).toBe('setup');
  expect(useTrack()).toBe('setup');
});
"#,
    );
    p
}

const ORDER: [&str; 5] = [
    "--reporter",
    "json",
    "src/a.test.ts",
    "src/b.test.ts",
    "src/c.test.ts",
];

#[test]
fn reused_isolate_restores_setup_mocks_and_invalidates_importers() {
    let p = reuse_project(
        "sa-reuse-config",
        "export default { test: { isolate: false, setupFiles: ['./setup.ts'] } };\n",
    );
    let r = turbo_test(&p.dir, &ORDER, &[("TURBO_JOBS", "1")]);
    assert_eq!(
        files(&r),
        vec![
            f("a.test.ts", "passed", 2, 0),
            f("b.test.ts", "passed", 1, 0),
            f("c.test.ts", "passed", 1, 0),
        ],
        "{}",
        r.all()
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn reuse_env_switches_force_reuse_on_and_off() {
    // A global written by one file is visible to the next only on a reused isolate.
    let mk = |name: &str, cfg: &str| {
        let p = Project::new(name);
        p.with_esbuild();
        p.file("package.json", r#"{ "name": "leak" }"#);
        p.file("vitest.config.ts", cfg);
        p.file(
            "a.test.ts",
            "globalThis.LEAK = (globalThis.LEAK || 0) + 1;\ntest('first', () => expect(globalThis.LEAK).toBe(1));\n",
        );
        p.file(
            "b.test.ts",
            "globalThis.LEAK = (globalThis.LEAK || 0) + 1;\ntest('second', () => expect(globalThis.LEAK).toBe(2));\n",
        );
        p
    };
    let args = ["--reporter", "json", "a.test.ts", "b.test.ts"];
    let on = mk("sa-reuse-env-on", "export default { test: {} };\n");
    let r = turbo_test(
        &on.dir,
        &args,
        &[("TURBO_JOBS", "1"), ("TURBO_REUSE_ISOLATE", "1")],
    );
    assert_eq!(
        files(&r),
        vec![
            f("a.test.ts", "passed", 1, 0),
            f("b.test.ts", "passed", 1, 0)
        ],
        "{}",
        r.all()
    );
    // isolate:false in the config, overridden by TURBO_NO_REUSE: each file is fresh
    let off = mk(
        "sa-reuse-env-off",
        "export default { test: { isolate: false } };\n",
    );
    let r = turbo_test(
        &off.dir,
        &args,
        &[("TURBO_JOBS", "1"), ("TURBO_NO_REUSE", "1")],
    );
    assert_eq!(
        files(&r),
        vec![
            f("a.test.ts", "passed", 1, 0),
            f("b.test.ts", "failed", 0, 1)
        ],
        "{}",
        r.all()
    );
}

#[test]
fn setup_mocks_without_the_module_runner_use_the_legacy_bundle() {
    // TURBO_NO_MR: the entry is esbuild-bundled with the setup mock's specifier externalized
    // and rewritten to the mock's absolute path, so the setup mock still intercepts.
    let p = Project::new("sa-legacy");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "legacy" }"#);
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./setup.ts'] } };\n",
    );
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "~/*": ["./*"] } } }"#,
    );
    p.file("analytics.ts", "export const track = () => 'real';\n");
    p.file(
        "hook.ts",
        "import { track } from './analytics';\nexport const useTrack = () => track();\n",
    );
    p.file(
        "setup.ts",
        "import { vi } from 'vitest';\nvi.mock('./analytics', () => { return { track: () => 'setup' }; });\nvi.mock('./icons/*?url', () => { return {}; });\nvi.mock('./partial', async (importOriginal) => { return { ...(await importOriginal()) }; });\n",
    );
    p.file("partial.ts", "export const p = 1;\n");
    p.file(
        "a.test.ts",
        "import { useTrack } from '~/hook';\ntest('setup mock under the legacy bundle', () => expect(useTrack()).toBe('setup'));\n",
    );
    for env in [
        vec![("TURBO_NO_MR", "1")],
        vec![("TURBO_NO_MR", "1"), ("TURBO_ESBUILD_DEBUG", "1")],
    ] {
        let r = turbo_test(&p.dir, &["--reporter", "json"], &env);
        assert_eq!(
            files(&r),
            vec![f("a.test.ts", "passed", 1, 0)],
            "{env:?}\n{}",
            r.all()
        );
    }
    // TURBO_NO_BUNDLE: no esbuild bundle at all — the native per-module ESM loader runs it
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_NO_MR", "1"), ("TURBO_NO_BUNDLE", "1")],
    );
    assert_eq!(
        files(&r),
        vec![f("a.test.ts", "passed", 1, 0)],
        "{}",
        r.all()
    );
}

#[test]
fn a_setup_file_that_fails_to_bundle_fails_the_test_file() {
    // A setup file that does not compile fails the test file with its error (vitest;
    // turbo-test#38 item 3 — it used to be ignored and the test ran without it).
    let p = Project::new("sa-setup-broken");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "broken" }"#);
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./good.ts', './broken.ts'] } };\n",
    );
    p.file("good.ts", "globalThis.GOOD_SETUP = 'yes';\n");
    p.file("broken.ts", "globalThis.BROKEN_SETUP = ;\n");
    p.file("a.test.ts", "test('never runs', () => {});\n");
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(
        files(&r),
        vec![f("a.test.ts", "error", 0, 0)],
        "{}",
        r.all()
    );
    let text = turbo_test(&p.dir, &[], &[]);
    assert!(
        text.stdout.lines().any(|l| l.starts_with("ERROR")
            && l.contains("setup file")
            && l.contains("broken.ts")
            // vite's oxc report for the file that does not transform (turbo-test#113)
            && l.contains("Error: Transform failed with 1 error:")),
        "{}",
        text.all()
    );
    // no half-written temp bundle is left behind in the cache
    let leftovers: Vec<_> = std::fs::read_dir(p.path(".tt-cache"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_bundled_setup_file_shares_the_test_files_package_instances() {
    // A setup file the CommonJS transform cannot take (top-level await) is bundled with its
    // packages external; each package it imports is loaded on demand by the module runner's
    // loaders, so the setup and the test file see one instance (and Node's namespace of a
    // CommonJS package). Expected: vitest 4.1.12 on the same project.
    let p = Project::new("ra-setup-bundle-externals");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"ra-setup-bundle-externals","type":"module"}"#,
    );
    p.file(
        "node_modules/pk/package.json",
        r#"{"name":"pk","main":"i.js"}"#,
    );
    p.file(
        "node_modules/pk/i.js",
        "exports.obj = { id: 'pk' }; exports.n = 1;\n",
    );
    p.file(
        "node_modules/pk2/package.json",
        r#"{"name":"pk2","main":"i.js"}"#,
    );
    p.file("node_modules/pk2/i.js", "exports.obj2 = { id: 'pk2' };\n");
    // a workspace package: node_modules/ws links to packages/ws, outside node_modules
    p.file("packages/ws/package.json", r#"{"name":"ws","main":"i.js"}"#);
    p.file("packages/ws/i.js", "exports.wsObj = { id: 'ws' };\n");
    std::os::unix::fs::symlink(p.path("packages/ws"), p.path("node_modules/ws")).unwrap();
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./setup.ts'] } };\n",
    );
    p.file(
        "setup.ts",
        "import { obj } from 'pk';\nimport * as ns2 from 'pk2';\nimport { wsObj } from 'ws';\nawait Promise.resolve();\n\
         (globalThis as any).__setup = { obj, ns2keys: Object.keys(ns2), obj2: ns2.obj2, wsObj };\n",
    );
    p.file(
        "a.test.ts",
        "import { test, expect } from 'vitest';\nimport { obj } from 'pk';\nimport { obj2 } from 'pk2';\nimport { wsObj } from 'ws';\n\
         test('shared', () => {\n  const s = (globalThis as any).__setup;\n  \
         expect([s.obj === obj, s.obj2 === obj2, s.ns2keys, s.wsObj === wsObj]).toEqual([true, true, ['default', 'module.exports', 'obj2'], true]);\n});\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert!(r.stdout.contains("Tests  1 passed (1)"), "{}", r.all());
    assert_eq!(r.code, Some(0), "{}", r.all());
}

/// turbo-test#194: a bundled setup file (top-level await) sees its own vi.mock of a module it
/// imports, as vitest 4.1.12 hoists the setup's vi.mock above its imports: vitest on the same
/// project gives `['mocked', 'mocked']` (1 passed). turbo-test lazy-loaded the bundle's
/// externalized `./src/dep` (the real module) before draining the setup's mocks: `['real',
/// 'mocked']`.
#[test]
#[ignore = "turbo-test#194: fails until the fix"]
fn a_bundled_setup_file_imports_its_own_vi_mock() {
    let p = Project::new("ra-setup-bundle-own-mock");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"ra-setup-bundle-own-mock","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { setupFiles: ['./setup.ts'] } };\n",
    );
    p.file("src/dep.ts", "export const v = 'real';\n");
    p.file(
        "setup.ts",
        "import { vi } from 'vitest';\nimport { v } from './src/dep';\n\
         vi.mock('./src/dep', () => ({ v: 'mocked' }));\nawait Promise.resolve();\n\
         (globalThis as any).__setupV = v;\n",
    );
    p.file(
        "a.test.ts",
        "import { expect, it } from 'vitest';\nimport { v } from './src/dep';\n\
         it('setup mock', () => { expect([(globalThis as any).__setupV, v]).toEqual(['mocked', 'mocked']); });\n",
    );
    for env in [&[][..], &[("TURBO_NATIVE_CJS", "0"), ("TURBO_NATIVE_DEPS", "0")][..]] {
        let r = turbo_test(&p.dir, &[], env);
        assert!(r.stdout.contains("Tests  1 passed (1)"), "{env:?}\n{}", r.all());
        assert_eq!(r.code, Some(0), "{env:?}\n{}", r.all());
    }
}
