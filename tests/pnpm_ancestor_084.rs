//! Opt-in pnpm ancestry and real-package provenance (#278).
#![cfg(unix)]
mod common;
use common::{turbo_test, Project};

fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"pnpm-optin"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p
}
fn package(p: &Project, base: &str, name: &str, value: &str) {
    p.file(
        &format!("{base}/{name}/package.json"),
        &format!(r#"{{"name":"{name}","main":"index.cjs"}}"#),
    );
    p.file(
        &format!("{base}/{name}/index.cjs"),
        &format!("module.exports = {{ origin: {value:?} }};"),
    );
}
fn node_control(p: &Project, source: &str) {
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(source)
        .arg(&p.dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Node control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn hoist(p: &Project, name: &str, version: &str) {
    let source = p
        .dir
        .join(format!("node_modules/.pnpm/{version}/node_modules/{name}"));
    let target = p
        .dir
        .join(format!("node_modules/.pnpm/node_modules/{name}"));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(source, target).unwrap();
}

#[test]
fn optin_follows_the_selected_pnpm_hoist_and_does_not_leak_memoized_flag_values() {
    let p = project("pnpm-selected");
    package(
        &p,
        "node_modules/.pnpm/pkg@1/node_modules",
        "pkg",
        "selected-one",
    );
    package(
        &p,
        "node_modules/.pnpm/pkg@9/node_modules",
        "pkg",
        "wrong-nine",
    );
    package(
        &p,
        "node_modules/.pnpm/scoped@1/node_modules",
        "@scope/pkg",
        "scoped-one",
    );
    package(
        &p,
        "node_modules/.pnpm/unlinked@1/node_modules",
        "unlinked",
        "must-not-load",
    );
    p.file(
        "node_modules/.pnpm/pkg@1/node_modules/pkg/package.json",
        r#"{"name":"pkg","exports":{".":"./index.cjs"}}"#,
    );
    hoist(&p, "pkg", "pkg@1");
    hoist(&p, "@scope/pkg", "scoped@1");
    node_control(
        &p,
        r#"
const assert = require('node:assert/strict');
const { createRequire } = require('node:module');
const path = require('node:path');
const root = process.argv[1];
const local = createRequire(path.join(root, 'package.json'));
assert.throws(() => local.resolve('pkg'), { code: 'MODULE_NOT_FOUND' });
const hoisted = createRequire(path.join(root, 'node_modules/.pnpm/package.json'));
assert.equal(hoisted('pkg').origin, 'selected-one');
assert.equal(hoisted('@scope/pkg').origin, 'scoped-one');
assert.throws(() => hoisted('pkg/private'), { code: 'ERR_PACKAGE_PATH_NOT_EXPORTED' });
assert.throws(() => hoisted('unlinked'), { code: 'MODULE_NOT_FOUND' });
"#,
    );
    p.file(
        "nested/flags.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('the opt-in follows actual hoist links and resolution memoization follows the flag', () => {
  delete process.env.TURBO_PNPM_ANCESTOR;
  assert.throws(() => require.resolve('pkg'), /Cannot find module/);
  process.env.TURBO_PNPM_ANCESTOR = '0';
  assert.throws(() => require.resolve('pkg'), /Cannot find module/);
  process.env.TURBO_PNPM_ANCESTOR = '1';
  assert.equal(require('pkg').origin, 'selected-one');
  assert.equal(require('@scope/pkg').origin, 'scoped-one');
  assert.throws(() => require('pkg/private'), /Cannot find module/);
  assert.throws(() => require('unlinked'), /Cannot find module/);
  delete process.env.TURBO_PNPM_ANCESTOR;
  assert.throws(() => require.resolve('pkg'), /Cannot find module/);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--jobs", "1"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn normal_packages_win_and_missing_tslib_and_hey_listen_have_no_shadow_modules() {
    let p = project("pnpm-real-packages");
    package(&p, "node_modules", "pkg", "normal-install");
    package(&p, "node_modules/.pnpm/node_modules", "pkg", "must-not-win");
    p.file("packages.test.cjs", r#"
const assert = require('node:assert/strict');
test('normal installation is preferred and missing dependencies report real errors', () => {
  assert.equal(require('pkg').origin, 'normal-install');
  for (const spec of ['tslib', 'hey-listen']) assert.throws(() => require(spec), /Cannot find module/);
});
"#);
    let r = turbo_test(&p.dir, &["--jobs", "1"], &[("TURBO_PNPM_ANCESTOR", "1")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn installed_helper_dependencies_return_their_actual_file_exports() {
    let p = project("pnpm-helper-packages");
    package(&p, "node_modules", "tslib", "installed-tslib-entry");
    package(
        &p,
        "node_modules",
        "hey-listen",
        "installed-hey-listen-entry",
    );
    p.file(
        "helpers.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('installed packages supply exports rather than runtime substitutes', () => {
  assert.equal(require('tslib').origin, 'installed-tslib-entry');
  assert.equal(require('hey-listen').origin, 'installed-hey-listen-entry');
  assert.equal(require.resolve('tslib').includes('node_modules/tslib/index.cjs'), true);
  assert.equal(require.resolve('hey-listen').includes('node_modules/hey-listen/index.cjs'), true);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--jobs", "1"], &[("TURBO_PNPM_ANCESTOR", "1")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}
