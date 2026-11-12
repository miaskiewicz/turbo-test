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

/// jest runs globalSetup and globalTeardown in one context: teardown reads what setup left on
/// `globalThis` (a server / container handle). A failing teardown fails the run (exit 1).
#[test]
fn global_teardown_sees_setup_globals_and_its_failure_fails_the_run() {
    for (teardown, code) in [
        (
            "module.exports = () => { globalThis.__server.stop(); };\n",
            Some(0),
        ),
        (
            "module.exports = () => { throw new Error('teardown boom'); };\n",
            Some(1),
        ),
    ] {
        let p = Project::new("jest-global-shared");
        p.with_esbuild();
        p.file("package.json", r#"{ "name": "globalsetup" }"#);
        p.file(
            "jest.config.js",
            "module.exports = { testEnvironment: 'node', globalSetup: './setup.js', globalTeardown: './teardown.js' };\n",
        );
        p.file(
            "setup.js",
            "const fs = require('fs');\nmodule.exports = () => { globalThis.__server = { stop() { fs.writeFileSync(__dirname + '/stopped.txt', 'yes'); } }; };\n",
        );
        p.file("teardown.js", teardown);
        p.file("a.test.js", "test('t', () => {});\n");
        let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
        assert_eq!(r.code, code, "{}", r.all());
        if code == Some(0) {
            assert!(p.path("stopped.txt").exists(), "{}", r.all());
        } else {
            assert!(r.stderr.contains("teardown boom"), "{}", r.all());
        }
    }
}

/// globalSetup's `process.env` writes reach the test files as Node stores them: values coerced to
/// strings, deleted keys removed.
#[test]
fn global_setup_env_is_coerced_and_deletions_apply() {
    let p = Project::new("jest-global-env");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "globalsetup" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', globalSetup: './setup.js' };\n",
    );
    p.file(
        "setup.js",
        "module.exports = () => { process.env.TT_PORT = 4000; process.env.TT_FLAG = true; delete process.env.TT_TO_DELETE; };\n",
    );
    p.file(
        "a.test.js",
        "test('env', () => expect([process.env.TT_PORT, process.env.TT_FLAG, process.env.TT_TO_DELETE]).toEqual(['4000', 'true', undefined]));\n",
    );
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1"],
        &[("TT_TO_DELETE", "x")],
    );
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

/// moduleNameMapper as jest evaluates it, within what reading the config text can know: a key
/// whose value is computed (`path.join(...)`) is skipped instead of breaking the specifiers it
/// matches; literal entries next to a spread still apply (with a warning that the spread is not
/// read); template literals; ECMAScript lookaround; `require.resolve('pkg')` is resolved from the
/// config's directory.
#[test]
fn module_name_mapper_handles_what_jest_configs_write() {
    let p = Project::new("jest-modulenamemapper-forms");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "mapper" }"#);
    p.file(
        "jest.config.js",
        r#"const path = require('path');
const extra = { '^extra$': '<rootDir>/extra.js' };
module.exports = {
    testEnvironment: 'node',
    moduleNameMapper: {
        ...extra,
        '^real$': path.join(__dirname, 'computed.js'),
        '^tpl$': `<rootDir>/tpl.js`,
        '^(?!keep)(\\w+)-mapped$': '<rootDir>/mapped.js',
        '^cfg$': require.resolve('cfgpkg'),
    },
};
"#,
    );
    p.file(
        "node_modules/real/package.json",
        r#"{ "name": "real", "main": "i.js" }"#,
    );
    p.file("node_modules/real/i.js", "module.exports = 'real';\n");
    p.file("tpl.js", "module.exports = 'tpl';\n");
    p.file("mapped.js", "module.exports = 'mapped';\n");
    p.file(
        "node_modules/keep-mapped/package.json",
        r#"{ "name": "keep-mapped", "main": "i.js" }"#,
    );
    p.file(
        "node_modules/keep-mapped/i.js",
        "module.exports = 'kept';\n",
    );
    p.file(
        "node_modules/cfgpkg/package.json",
        r#"{ "name": "cfgpkg", "main": "i.js" }"#,
    );
    p.file("node_modules/cfgpkg/i.js", "module.exports = 'root cfg';\n");
    p.file(
        "sub/node_modules/cfgpkg/package.json",
        r#"{ "name": "cfgpkg", "main": "i.js" }"#,
    );
    p.file(
        "sub/node_modules/cfgpkg/i.js",
        "module.exports = 'shadowed cfg';\n",
    );
    p.file(
        "sub/a.test.js",
        r#"
test('jest config forms', () => {
    expect(require('real')).toBe('real');
    expect(require('tpl')).toBe('tpl');
    expect(require('foo-mapped')).toBe('mapped');
    expect(require('keep-mapped')).toBe('kept');
    expect(require('cfg')).toBe('root cfg');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
    assert!(
        r.stderr.contains("moduleNameMapper") && r.stderr.contains("spread"),
        "warns about the unread spread: {}",
        r.all()
    );
}

/// The shapes a global script takes: a package's script named by module (`pkg/setup`), an ES
/// module's `export default`, a script with side effects only (no function to call), and one that
/// fails to load (exit 1, its error on stderr).
#[test]
fn global_scripts_by_module_name_default_export_side_effect_and_load_failure() {
    let cases: [(&str, &str, &str, Option<i32>); 4] = [
        (
            "setup-pkg/setup",
            "node_modules/setup-pkg/setup.js",
            "module.exports = () => { process.env.TT_SETUP = 'pkg'; };\n",
            Some(0),
        ),
        (
            "./setup.js",
            "setup.js",
            "module.exports = { default: async () => { process.env.TT_SETUP = 'default'; } };\n",
            Some(0),
        ),
        (
            "./setup.js",
            "setup.js",
            "process.env.TT_SETUP = 'side effect';\n",
            Some(0),
        ),
        (
            "./setup.js",
            "setup.js",
            "require('./missing-dep');\n",
            Some(1),
        ),
    ];
    for (i, (config_path, file, body, code)) in cases.iter().enumerate() {
        let p = Project::new(&format!("jest-global-shapes-{i}"));
        p.with_esbuild();
        p.file("package.json", r#"{ "name": "globalsetup" }"#);
        p.file(
            "jest.config.js",
            &format!(
                "module.exports = {{ testEnvironment: 'node', globalSetup: '{config_path}' }};\n"
            ),
        );
        p.file(file, body);
        p.file(
            "a.test.js",
            "test('ran', () => expect(typeof process.env.TT_SETUP).toBe('string'));\n",
        );
        let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
        assert_eq!(r.code, *code, "case {i}: {}", r.all());
        if *code == Some(1) {
            assert!(
                r.stderr.contains("globalSetup failed") && r.stderr.contains("missing-dep"),
                "case {i}: {}",
                r.all()
            );
        }
    }
}

/// A `$` not followed by a group number stays literal in a target; a mapper the config builds in
/// code (`moduleNameMapper: mapper`) cannot be read from its text and is reported, not silently
/// dropped.
#[test]
fn module_name_mapper_literal_dollar_and_unreadable_mapper() {
    let p = Project::new("jest-modulenamemapper-dollar");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "mapper" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', moduleNameMapper: { '^dollar$': '<rootDir>/$lib.js' } };\n",
    );
    p.file("$lib.js", "module.exports = 'dollar';\n");
    p.file(
        "a.test.js",
        "test('$', () => expect(require('dollar')).toBe('dollar'));\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());

    let q = Project::new("jest-modulenamemapper-variable");
    q.with_esbuild();
    q.file("package.json", r#"{ "name": "mapper" }"#);
    q.file(
        "jest.config.js",
        "const mapper = { '^x$': '<rootDir>/x.js' };\nmodule.exports = { testEnvironment: 'node', moduleNameMapper: mapper };\n",
    );
    q.file("a.test.js", "test('t', () => {});\n");
    let r = turbo_test(&q.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
    assert!(
        r.stderr.contains("moduleNameMapper") && r.stderr.contains("not an object literal"),
        "{}",
        r.all()
    );

    // a mapper whose text does not parse (here a stray token) is reported too
    let b = Project::new("jest-modulenamemapper-broken");
    b.with_esbuild();
    b.file("package.json", r#"{ "name": "mapper" }"#);
    b.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', moduleNameMapper: { '^x$': '<rootDir>/x.js' %% } };\n",
    );
    b.file("a.test.js", "test('t', () => {});\n");
    let r = turbo_test(&b.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert!(r.stderr.contains("does not parse"), "{}", r.all());
}

/// jest loads globalSetup and globalTeardown with one require cache: a module both require is one
/// instance, so the handle setup's `start()` keeps is there for teardown's `stop()`.
#[test]
fn global_setup_and_teardown_share_module_instances() {
    let p = Project::new("jest-global-modules");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "globalsetup" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', globalSetup: './setup.js', globalTeardown: './teardown.js' };\n",
    );
    p.file(
        "server.js",
        "let handle;\nexports.start = () => { handle = { up: true }; };\nexports.stop = () => { if (!handle) throw new Error('stop: no server started'); handle = undefined; };\n",
    );
    p.file(
        "setup.js",
        "module.exports = () => require('./server').start();\n",
    );
    p.file(
        "teardown.js",
        "module.exports = () => require('./server').stop();\n",
    );
    p.file("a.test.js", "test('t', () => {});\n");
    let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(!r.stderr.contains("no server started"), "{}", r.all());
}
