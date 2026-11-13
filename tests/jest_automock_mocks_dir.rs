//! `jest.mock('./module')` without a factory loads a colocated `__mocks__/module.js` when
//! present (turbo-test#341), and `jest.onGenerateMock` callbacks are chained.
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
    p.file("package.json", r#"{ "name": "jestautomockmocks" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
}

// ---- Issue 1: colocated __mocks__ directory ----

#[test]
fn factory_less_jest_mock_loads_colocated_manual_mock() {
    // jest.mock('./greet') without a factory should load __mocks__/greet.js when it exists,
    // not generate a stub from the real module.
    let p = Project::new("jest-automock-mocks-dir-341");
    base_jest(&p);
    p.file("greet.js", "module.exports = { hello: () => 'real' };\n");
    p.file(
        "__mocks__/greet.js",
        "module.exports = { hello: () => 'manual-mock' };\n",
    );
    p.file(
        "a.test.js",
        "jest.mock('./greet');\n\
         const g = require('./greet');\n\
         test('uses a colocated manual mock', () => {\n\
             expect(g.hello()).toBe('manual-mock');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn factory_less_jest_mock_falls_back_to_automock_without_mocks_dir() {
    // When no __mocks__/greet.js exists, factory-less jest.mock still generates a stub.
    let p = Project::new("jest-automock-no-mocks-dir-341");
    base_jest(&p);
    p.file(
        "greet.js",
        "module.exports = { hello: () => 'real', name: 'alice' };\n",
    );
    p.file(
        "a.test.js",
        "jest.mock('./greet');\n\
         const g = require('./greet');\n\
         test('automocked stub', () => {\n\
             expect(jest.isMockFunction(g.hello)).toBe(true);\n\
             expect(g.hello()).toBe(undefined);\n\
             expect(g.name).toBe('alice');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn automock_require_intercept_loads_colocated_manual_mock() {
    // With enableAutomock(), require('./greet') should load __mocks__/greet.js when present.
    let p = Project::new("jest-automock-require-mocks-341");
    base_jest(&p);
    p.file("greet.js", "module.exports = { hello: () => 'real' };\n");
    p.file(
        "__mocks__/greet.js",
        "module.exports = { hello: () => 'manual-mock' };\n",
    );
    p.file(
        "a.test.js",
        "jest.enableAutomock();\n\
         const g = require('./greet');\n\
         test('require loads manual mock', () => {\n\
             expect(g.hello()).toBe('manual-mock');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn colocated_manual_mock_in_subdirectory() {
    // __mocks__ next to the module in a subdirectory.
    let p = Project::new("jest-automock-subdir-mocks-341");
    base_jest(&p);
    p.file(
        "lib/math.js",
        "module.exports = { add: (a, b) => a + b };\n",
    );
    p.file(
        "lib/__mocks__/math.js",
        "module.exports = { add: () => 42 };\n",
    );
    p.file(
        "a.test.js",
        "jest.mock('./lib/math');\n\
         const m = require('./lib/math');\n\
         test('manual mock from subdir', () => {\n\
             expect(m.add()).toBe(42);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

// ---- Issue 2: onGenerateMock callback chaining ----

#[test]
fn on_generate_mock_chains_multiple_callbacks() {
    // Two onGenerateMock calls: each receives the module path and the current mock.
    // The second callback receives the result of the first.
    let p = Project::new("jest-automock-chain-341");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "jest.onGenerateMock((path, mock) => {\n\
             mock.tag1 = path;\n\
             return mock;\n\
         });\n\
         jest.onGenerateMock((path, mock) => {\n\
             mock.tag2 = 'chained';\n\
             return mock;\n\
         });\n\
         jest.enableAutomock();\n\
         const m = require('./m');\n\
         test('passes the module path and mock', () => {\n\
             expect(typeof m.tag1).toBe('string');\n\
             expect(m.tag1).toContain('/m.js');\n\
             expect(m.tag2).toBe('chained');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn on_generate_mock_receives_module_path() {
    // A single onGenerateMock callback receives (path, mock) where path is the resolved path.
    let p = Project::new("jest-automock-path-arg-341");
    base_jest(&p);
    p.file("m.js", "module.exports = { f: () => 'real' };\n");
    p.file(
        "a.test.js",
        "let receivedPath = null;\n\
         jest.onGenerateMock((path, mock) => {\n\
             receivedPath = path;\n\
             return mock;\n\
         });\n\
         jest.enableAutomock();\n\
         const m = require('./m');\n\
         test('path is the resolved module path', () => {\n\
             expect(typeof receivedPath).toBe('string');\n\
             expect(receivedPath).toContain('/m.js');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

/// jest passes every automock it generates through the `onGenerateMock` callbacks, in order —
/// the one a factory-less `jest.mock('./x')` makes too, not only `enableAutomock`'s
/// (turbo-test#341; the #315 repro kit's jest-edge probe).
#[test]
fn on_generate_mock_callbacks_shape_a_factory_less_jest_mock() {
    let p = Project::new("jest-automock-ongenerate-factoryless");
    base_jest(&p);
    p.file("edge-generated.js", "exports.load = () => 7;\n");
    p.file(
        "a.test.js",
        r#"test('callbacks run in order on the generated mock', () => {
  const seen = [];
  jest.onGenerateMock((path, mock) => {
    seen.push(path.endsWith('edge-generated.js'));
    return { ...mock, first: true };
  });
  jest.onGenerateMock((_path, mock) => ({ ...mock, second: true }));
  jest.mock('./edge-generated.js');
  const generated = require('./edge-generated.js');
  expect(seen).toEqual([true]);
  expect([generated.first, generated.second]).toEqual([true, true]);
  expect(jest.isMockFunction(generated.load)).toBe(true);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
