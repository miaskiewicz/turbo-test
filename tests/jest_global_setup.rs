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

/// jest's globalSetup: the path may use `<rootDir>` (the config's rootDir) or be rootDir-relative;
/// the function gets `(globalConfig, projectConfig)`; and `process.env` it sets reaches the test
/// files (jest runs it in the parent before forking workers: @shelf/jest-mongodb's MONGO_URL).
#[test]
fn global_setup_resolves_root_dir_and_shares_process_env() {
    for path in ["<rootDir>/../config/setup.js", "../config/setup.js"] {
        let p = Project::new("jest-globalsetup-rootdir");
        p.with_esbuild();
        p.file("package.json", r#"{ "name": "globalsetup" }"#);
        p.file(
            "jest.config.js",
            &format!(
                "module.exports = {{ rootDir: 'src', testEnvironment: 'node', globalSetup: '{}' }};\n",
                path
            ),
        );
        p.file(
            "config/setup.js",
            "module.exports = async (globalConfig, projectConfig) => {\n\
               process.env.FROM_GLOBAL_SETUP = 'yes';\n\
               process.env.ROOT_DIRS = [globalConfig.rootDir, projectConfig.rootDir].join('|');\n\
             };\n",
        );
        p.file(
            "src/a.test.js",
            "test('globalSetup env', () => {\n\
               expect(process.env.FROM_GLOBAL_SETUP).toBe('yes');\n\
               const [g, pr] = process.env.ROOT_DIRS.split('|');\n\
               expect(g.endsWith('/src')).toBe(true);\n\
               expect(pr).toBe(g);\n\
             });\n",
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
        assert_eq!(counts(&r), (1, 0), "globalSetup {path}: {}", r.all());
    }
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

/// jest's moduleNameMapper keys are regexes, tried in order (first match wins); `$n` is the n-th
/// capture; a target may be a list (the first that resolves) or a `require.resolve(...)` call.
#[test]
fn module_name_mapper_matches_regexes_in_order() {
    let p = Project::new("jest-modulenamemapper-regex");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "mapper" }"#);
    p.file(
        "jest.config.js",
        r#"module.exports = {
    testEnvironment: 'node',
    moduleNameMapper: {
        '^lodash-es$': '<rootDir>/src/lodash.js',
        '\\.(css|less)$': '<rootDir>/styleMock.js',
        '^@/(.*)\\.svg$': '<rootDir>/svgMock.js',
        '^@/(.*)$': '<rootDir>/src/$1',
        '^v\\d{1,2}$': require.resolve('./v.js'),
        '^cfg/(\\w+)$': ['<rootDir>/missing/$1.js', '<rootDir>/config/$1.js'],
    },
};
"#,
    );
    p.file("src/lodash.js", "module.exports = 'lodash';\n");
    p.file("styleMock.js", "module.exports = 'style';\n");
    p.file("svgMock.js", "module.exports = 'svg';\n");
    p.file("src/thing.js", "module.exports = 'thing';\n");
    p.file("v.js", "module.exports = 'v';\n");
    p.file("config/dev.js", "module.exports = 'dev';\n");
    p.file(
        "a.test.js",
        r#"
test('regex keys, order, captures, lists, require.resolve', () => {
    expect(require('lodash-es')).toBe('lodash');
    expect(require('./theme.less')).toBe('style');
    expect(require('@/logo.svg')).toBe('svg');
    expect(require('@/thing')).toBe('thing');
    expect(require('v12')).toBe('v');
    expect(require('cfg/dev')).toBe('dev');
    expect(() => require('lodash-es/fp')).toThrow();
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
