//! Config switches vitest reads for every file of the run: `server.deps.inline` (an inlined
//! package runs through the module runner, so its own imports see `vi.mock`; turbo-test#216) and
//! the `turbo-dom` environment name (the DOM is installed for every file, as for `jsdom`;
//! turbo-test#217). Expected values = vitest 4.1.12.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn assert_passes(r: &common::Run, n: u32) {
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains(&format!("Tests  {n} passed ({n})")),
        "{}",
        r.all()
    );
}

/// `node_modules/zzds` re-exports a value of `node_modules/zzinner`; the test mocks `zzinner`.
fn inline_project(name: &str, config: &str, esbuild: bool) -> Project {
    let p = Project::new(name);
    if esbuild {
        p.with_esbuild();
    }
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file("vitest.config.ts", config);
    for (pkg, esm, cjs) in [
        (
            "zzds",
            "import { A } from 'zzinner';\nexport const f = () => A;\n",
            "exports.f = () => require('zzinner').A;\n",
        ),
        (
            "zzinner",
            "export const A = 'real';\n",
            "exports.A = 'real';\n",
        ),
    ] {
        p.file(
            &format!("node_modules/{pkg}/package.json"),
            &format!(
                r#"{{"name":"{pkg}","exports":{{".":{{"import":"./index.mjs","require":"./index.js"}}}}}}"#
            ),
        );
        p.file(&format!("node_modules/{pkg}/index.mjs"), esm);
        p.file(&format!("node_modules/{pkg}/index.js"), cjs);
    }
    p
}

const MOCKED: &str = r#"import { f } from 'zzds';
import { vi } from 'vitest';
vi.mock('zzinner', () => ({ A: 'mocked' }));
test('an inlined package imports the mock', () => {
  expect(f()).toBe('mocked');
});
"#;

const REAL: &str = r#"import { f } from 'zzds';
import { vi } from 'vitest';
vi.mock('zzinner', () => ({ A: 'mocked' }));
test('an external package imports the real module', () => {
  expect(f()).toBe('real');
});
"#;

#[test]
fn an_inlined_package_sees_vi_mock_and_an_external_one_does_not() {
    for esbuild in [false, true] {
        for (i, (config, file, body)) in [
            (
                "export default { test: { server: { deps: { inline: ['zzds'] } } } };",
                "a.test.ts",
                MOCKED,
            ),
            (
                "export default { test: { server: { deps: { inline: true } } } };",
                "a.test.ts",
                MOCKED,
            ),
            (
                "export default { test: { server: { deps: { inline: ['other'] } } } };",
                "a.test.ts",
                REAL,
            ),
            ("export default { test: {} };", "a.test.ts", REAL),
        ]
        .into_iter()
        .enumerate()
        {
            let p = inline_project(&format!("rc-deps-inline-{esbuild}-{i}"), config, esbuild);
            p.file(file, body);
            let r = turbo_test(&p.dir, &[file], &[]);
            assert_eq!(r.code, Some(0), "esbuild={esbuild} {config}\n{}", r.all());
            assert_passes(&r, 1);
        }
    }
}

#[test]
fn the_turbo_dom_environment_installs_the_dom_for_every_file() {
    let p = Project::new("rc-env-turbo-dom");
    p.file("package.json", r#"{"name":"rc-env-turbo-dom"}"#);
    p.file(
        "vitest.config.ts",
        "export default { test: { globals: true, environment: 'turbo-dom', setupFiles: ['./setup.js'] } };",
    );
    // a setup-file hook that reads the window (as @testing-library/user-event's clipboard reset
    // does), in a test file that names no DOM API
    p.file(
        "setup.js",
        "afterEach(() => { globalThis.window.navigator; });\n",
    );
    p.file(
        "plain.test.js",
        "test('a pure-logic file', () => { expect(typeof globalThis.window).toBe('object'); });\n",
    );
    assert_passes(&turbo_test(&p.dir, &["plain.test.js"], &[]), 1);
}
