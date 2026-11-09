//! `jest.enableAutomock()` / `jest.disableAutomock()` / `autoMockOn` / `autoMockOff` /
//! `jest.onGenerateMock(factory)` and jest config `automock: true` (turbo-test#184).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
    )
}

fn base_jest(p: &Project) {
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestautomock" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
}

#[test]
fn enable_automock_replaces_local_module_functions_with_spies() {
    let p = Project::new("jest-automock-enable-184");
    base_jest(&p);
    p.file(
        "greet.js",
        "module.exports = { hello: () => 'real', name: 'alice' };\n",
    );
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         const g = require('./greet');\n\
         test('automocked', () => {\n\
             expect(jest.isMockFunction(g.hello)).toBe(true);\n\
             expect(g.hello()).toBe(undefined);\n\
             expect(g.name).toBe('alice');\n\
             g.hello.mockReturnValue('mocked');\n\
             expect(g.hello()).toBe('mocked');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn disable_automock_restores_real_behavior() {
    let p = Project::new("jest-automock-disable-184");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         jest.disableAutomock();\n\
         const m = require('./m');\n\
         test('real', () => expect(m.f()).toBe('real'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn jest_config_automock_true_enables_at_bootstrap() {
    let p = Project::new("jest-automock-config-184");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestautomockcfg" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', automock: true };\n",
    );
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "const m = require('./m');\n\
         test('bootstrap', () => {\n\
             expect(jest.isMockFunction(m.f)).toBe(true);\n\
             expect(m.f()).toBe(undefined);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn jest_unmock_opts_specific_module_out() {
    let p = Project::new("jest-automock-unmock-184");
    base_jest(&p);
    p.file("a.js", "module.exports = { f: () => 'ra' };\n");
    p.file("b.js", "module.exports = { f: () => 'rb' };\n");
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         jest.unmock('./a');\n\
         const a = require('./a');\n\
         const b = require('./b');\n\
         test('a real, b mocked', () => {\n\
             expect(a.f()).toBe('ra');\n\
             expect(jest.isMockFunction(b.f)).toBe(true);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn on_generate_mock_replaces_the_default_generator() {
    let p = Project::new("jest-automock-ongen-184");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "jest.onGenerateMock(() => ({ custom: true }));\n\
         jest.enableAutomock();\n\
         const m = require('./m');\n\
         test('custom', () => expect(m).toEqual({ custom: true }));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn automock_off_alias_disables() {
    let p = Project::new("jest-automock-alias-184");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "jest.autoMockOn();\n\
         jest.autoMockOff();\n\
         const m = require('./m');\n\
         test('alias off', () => expect(m.f()).toBe('real'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn automock_state_does_not_leak_across_files() {
    // File A enables automock and installs a custom generator; file B must start on real
    // timers of automock: it should see the REAL module (state is per-file in jest).
    let p = Project::new("jest-automock-cross-file-184");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         jest.onGenerateMock(() => ({ custom: 'from-a' }));\n\
         const m = require('./m');\n\
         test('a: custom gen', () => expect(m).toEqual({ custom: 'from-a' }));\n",
    );
    p.file(
        "b.test.js",
        "const m = require('./m');\n\
         test('b: real, not leaked from a', () => {\n\
             expect(m.f()).toBe('real');\n\
             expect(jest.isMockFunction(m.f)).toBe(false);\n\
         });\n",
    );
    // Force a single worker so both files share one runtime state (the leak path).
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (2, 0), "{}", r.all());
}

#[test]
fn node_modules_modules_are_not_automocked() {
    let p = Project::new("jest-automock-nm-184");
    base_jest(&p);
    // A fake third-party package under node_modules. Automock must NOT fire on it.
    p.file(
        "node_modules/vendor/package.json",
        r#"{ "name": "vendor", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/vendor/index.js",
        "module.exports = { vf: () => 'vendor-real' };\n",
    );
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         const v = require('vendor');\n\
         test('not automocked', () => {\n\
             expect(v.vf()).toBe('vendor-real');\n\
             expect(jest.isMockFunction(v.vf)).toBe(false);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
