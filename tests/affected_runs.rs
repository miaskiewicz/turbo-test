#![cfg(unix)]
mod common;
use common::{turbo_test, Project};

fn run(p: &Project) -> common::Run {
    turbo_test(&p.dir, &["--since-last-run", "--jobs", "1"], &[])
}

#[test]
fn dependencies_failures_and_setup_in_each_mode() {
    for (mode, config_name, config) in [
        (
            "backend",
            "vitest.config.js",
            "export default { test: { environment: 'node', setupFiles: ['./setup.js'] } }",
        ),
        (
            "frontend",
            "vitest.config.js",
            "export default { test: { environment: 'jsdom', setupFiles: ['./setup.js'] } }",
        ),
        (
            "vitest",
            "vitest.config.js",
            "export default { test: { environment: 'node', setupFiles: ['./setup.js'] } }",
        ),
        (
            "jest",
            "jest.config.js",
            "module.exports = { testEnvironment: 'node', setupFiles: ['./setup.js'] }",
        ),
    ] {
        let p = Project::new(mode);
        p.with_esbuild();
        p.file(config_name, config);
        p.file("setup.js", "globalThis.setupValue = 1;");
        p.file("dep.js", "export const value = 1;");
        p.file(
            "a.test.js",
            "import { value } from './dep.js'; test('a', () => expect(value).toBe(1));",
        );
        p.file(
            "b.test.js",
            "test('b', () => expect(globalThis.setupValue).toBe(1));",
        );
        let first = run(&p);
        assert_eq!(first.code, Some(0), "{mode}: {}", first.all());
        let unchanged = run(&p);
        assert!(
            unchanged.all().contains("no changed test files"),
            "{mode}: {}",
            unchanged.all()
        );
        p.file("dep.js", "export const value = 1; // changed");
        let changed = run(&p);
        assert_eq!(changed.code, Some(0), "{mode}: {}", changed.all());
        assert!(
            changed.all().contains("a.test.js"),
            "{mode}: {}",
            changed.all()
        );
        assert!(
            !changed.all().contains("b.test.js"),
            "{mode}: {}",
            changed.all()
        );
        p.file("setup.js", "globalThis.setupValue = 1; // changed");
        let setup = run(&p);
        assert!(
            setup.all().contains("a.test.js") && setup.all().contains("b.test.js"),
            "{mode}: {}",
            setup.all()
        );
        p.file("b.test.js", "test('failure', () => expect(1).toBe(2));");
        let failed = run(&p);
        assert_ne!(failed.code, Some(0));
        let retry = run(&p);
        assert_ne!(retry.code, Some(0));
        assert!(retry.all().contains("b.test.js"), "{mode}: {}", retry.all());
    }
}

#[test]
fn computed_dynamic_import_is_recorded() {
    let p = Project::new("dynamic-deps");
    p.with_esbuild();
    p.file("dep.js", "export const value = 1;");
    p.file("a.test.js", "test('dynamic', async () => { const name = './dep.js'; const x = await import(name); expect(x.value).toBe(1); });");
    let first = run(&p);
    assert_eq!(first.code, Some(0), "{}", first.all());
    p.file("dep.js", "export const value = 1; // changed");
    let changed = run(&p);
    assert_eq!(changed.code, Some(0), "{}", changed.all());
    assert!(changed.all().contains("a.test.js"), "{}", changed.all());
}

#[test]
fn partial_runs_do_not_establish_a_complete_baseline() {
    let p = Project::new("partial-run-history");
    p.with_esbuild();
    p.file("a.test.js", "test('one', () => {}); test('two', () => {});");
    let partial = turbo_test(&p.dir, &["--since-last-run", "-t", "one"], &[]);
    assert_eq!(partial.code, Some(0), "{}", partial.all());
    let complete = run(&p);
    assert_eq!(complete.code, Some(0), "{}", complete.all());
    assert!(complete.all().contains("a.test.js"), "{}", complete.all());
    assert!(run(&p).all().contains("no changed test files"));
}

#[test]
fn config_and_touch_changes_select_affected_tests() {
    let p = Project::new("config-history");
    p.with_esbuild();
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file("a.test.js", "test('one', () => {});");
    assert_eq!(run(&p).code, Some(0));
    assert!(run(&p).all().contains("no changed test files"));
    assert!(std::process::Command::new("touch")
        .arg(p.path("a.test.js"))
        .status()
        .unwrap()
        .success());
    assert!(run(&p).all().contains("a.test.js"));
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } }; // changed",
    );
    assert!(run(&p).all().contains("a.test.js"));
}

#[test]
fn deleted_dependency_and_jest_global_setup_changes_trigger_runs() {
    let p = Project::new("global-and-deleted-history");
    p.with_esbuild();
    p.file(
        "jest.config.js",
        "module.exports = { globalSetup: './global.js' };",
    );
    p.file("global.js", "module.exports = () => {}; // first");
    p.file("dep.js", "export const value = 1;");
    p.file(
        "a.test.js",
        "import { value } from './dep.js'; test('one', () => expect(value).toBe(1));",
    );
    assert_eq!(run(&p).code, Some(0));
    assert!(run(&p).all().contains("no changed test files"));
    p.file("global.js", "module.exports = () => {}; // edited");
    assert!(run(&p).all().contains("a.test.js"));
    std::fs::remove_file(p.path("dep.js")).unwrap();
    let deleted = run(&p);
    assert_ne!(deleted.code, Some(0));
    assert!(deleted.all().contains("a.test.js"), "{}", deleted.all());
}
