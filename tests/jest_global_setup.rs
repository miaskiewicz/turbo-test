//! Jest globalSetup / globalTeardown / moduleNameMapper (#335).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap_or(0),
        j["numFailedTests"].as_u64().unwrap_or(0),
    )
}

/// #335: globalSetup runs a JS file once before all test suites.
/// It can export a default function or a module.exports function.
#[test]
fn global_setup_runs_before_tests() {
    let p = Project::new("jest-globalsetup-335");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "globalsetup" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', globalSetup: './setup.js' };\n",
    );
    p.file(
        "setup.js",
        r#"
const fs = require('fs');
const path = require('path');
module.exports = async function () {
    fs.writeFileSync(path.join(__dirname, 'setup-ran.txt'), 'yes');
};
"#,
    );
    p.file(
        "a.test.js",
        r#"
const fs = require('fs');
const path = require('path');
test('globalSetup ran', () => {
    const flag = fs.readFileSync(path.join(__dirname, 'setup-ran.txt'), 'utf8');
    expect(flag).toBe('yes');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (1, 0),
        "globalSetup should run before tests: {}",
        r.all()
    );
}

/// #335: globalTeardown runs a JS file once after all test suites.
#[test]
fn global_teardown_runs_after_tests() {
    let p = Project::new("jest-globalteardown-335");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "globalteardown" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', globalTeardown: './teardown.js' };\n",
    );
    p.file(
        "teardown.js",
        r#"
const fs = require('fs');
const path = require('path');
module.exports = async function () {
    fs.writeFileSync(path.join(__dirname, 'teardown-ran.txt'), 'yes');
};
"#,
    );
    p.file(
        "a.test.js",
        "test('passes', () => { expect(1).toBe(1); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!((pass, fail), (1, 0), "test should pass: {}", r.all());
    let flag = std::fs::read_to_string(p.dir.join("teardown-ran.txt")).unwrap_or_default();
    assert_eq!(flag, "yes", "globalTeardown should have run after tests");
}

/// #335: moduleNameMapper maps regex patterns to module paths.
#[test]
fn module_name_mapper_resolves_aliases() {
    let p = Project::new("jest-modulenamemapper-335");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "mapper" }"#);
    p.file(
        "jest.config.js",
        r#"module.exports = {
    testEnvironment: 'node',
    moduleNameMapper: { '^@src/(.*)$': '<rootDir>/src/$1' }
};
"#,
    );
    p.file(
        "src/helper.js",
        "module.exports = { greet: () => 'hello' };\n",
    );
    p.file(
        "a.test.js",
        r#"
const { greet } = require('@src/helper');
test('moduleNameMapper resolves @src/', () => {
    expect(greet()).toBe('hello');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    let (pass, fail) = counts(&r);
    assert_eq!(
        (pass, fail),
        (1, 0),
        "moduleNameMapper should resolve @src/ alias: {}",
        r.all()
    );
}
