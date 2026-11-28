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

fn install_vite_graph_fixture(p: &Project) {
    p.file(
        "node_modules/vite/package.json",
        r#"{"name":"vite","type":"module","main":"index.mjs"}"#,
    );
    p.file("node_modules/vite/index.mjs", r#"
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
export async function createServer(options) {
  if (options.mode !== 'test' || !options.environments?.__vitest__) throw new Error('wrong Vite environment');
  const nodes = new Map();
  const env = {
    moduleGraph: { getModuleById: id => nodes.get(id) },
    pluginContainer: { watchFiles: new Set() },
    async transformRequest(url) {
      const file = url.startsWith('/@fs/') ? url.slice(5) : url;
      const imports = [...readFileSync(file, 'utf8').matchAll(/(?:from\s*|import\s*)['"](\.\/.+?)['"]/g)];
      const children = imports.map(match => ({ id: join(dirname(file), match[1]) }));
      if (file.endsWith('/dep.js') || file.endsWith('/dep.css')) children.push({ id: join(dirname(file), 'theme.json') });
      nodes.set(file, { importedModules: children });
    }
  };
  const globalFile = existsSync(join(process.cwd(), 'global-target')) ? 'theme.json' : 'global.watch';
  const client = { pluginContainer: { watchFiles: new Set(existsSync(join(process.cwd(), globalFile)) ? [join(process.cwd(), globalFile)] : []) } };
  return { environments: { __vitest__: env, client }, close: async () => {} };
}
"#);
}

#[test]
fn vite_watch_files_select_only_the_dependent_tests_without_git() {
    let p = Project::new("vite-watch-history");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("dep.js", "export const value = 1;");
    p.file("theme.json", "{}");
    p.file(
        "a.test.js",
        "import {value} from './dep.js'; test('a', () => expect(value).toBe(1));",
    );
    p.file("b.test.js", "test('b', () => {});");
    let first = run(&p);
    assert_eq!(first.code, Some(0), "{}", first.all());
    assert!(run(&p).all().contains("no changed test files"));
    p.file("theme.json", "{\"changed\":true}");
    let changed = run(&p);
    assert_eq!(changed.code, Some(0), "{}", changed.all());
    assert!(changed.all().contains("a.test.js"), "{}", changed.all());
    assert!(!changed.all().contains("b.test.js"), "{}", changed.all());
    assert!(run(&p).all().contains("no changed test files"));
}

#[test]
fn missing_vite_graph_never_marks_a_file_up_to_date() {
    let p = Project::new("missing-vite-watch");
    p.with_esbuild();
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("a.test.js", "test('a', () => {});");
    let first = run(&p);
    assert_eq!(first.code, Some(0), "{}", first.all());
    assert!(
        first.all().contains("graph collection") || first.all().contains("Vite helper"),
        "{}",
        first.all()
    );
    let next = run(&p);
    assert!(next.all().contains("a.test.js"), "{}", next.all());
}

#[test]
fn vite_watch_files_select_dependent_tests_with_git() {
    let p = Project::new("vite-watch-git");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file(".gitignore", "node_modules/\n.tt-cache/\n.tt-tmp/\n");
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("dep.js", "export const value = 1;");
    p.file("theme.json", "{}");
    p.file(
        "a.test.js",
        "import {value} from './dep.js'; test('a', () => expect(value).toBe(1));",
    );
    p.file("b.test.js", "test('b', () => {});");
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "base",
        ][..],
    ] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&p.dir)
            .status()
            .unwrap()
            .success());
    }
    p.file("theme.json", "{\"changed\":true}");
    let result = turbo_test(&p.dir, &["--changed", "--jobs", "1"], &[]);
    assert_eq!(result.code, Some(0), "{}", result.all());
    assert!(result.all().contains("a.test.js"), "{}", result.all());
    assert!(!result.all().contains("b.test.js"), "{}", result.all());
}

#[test]
fn vite_global_watch_files_rerun_all_tests() {
    let p = Project::new("vite-global-watch");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("global.watch", "before");
    p.file("a.test.js", "test('a', () => {});");
    p.file("b.test.js", "test('b', () => {});");
    assert_eq!(run(&p).code, Some(0));
    assert!(run(&p).all().contains("no changed test files"));
    p.file("global.watch", "after");
    let changed = run(&p);
    assert!(
        changed.all().contains("a.test.js") && changed.all().contains("b.test.js"),
        "{}",
        changed.all()
    );
}

#[test]
fn vite_watched_file_of_a_computed_dynamic_import_is_recorded() {
    let p = Project::new("vite-watch-dynamic");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("dep.js", "export const value = 1;");
    p.file("theme.json", "{}");
    p.file("a.test.js", "test('a', async () => { const n = './dep.js'; const m = await import(n); expect(m.value).toBe(1); });");
    p.file("b.test.js", "test('b', () => {});");
    let first = run(&p);
    assert_eq!(first.code, Some(0), "{}", first.all());
    assert!(run(&p).all().contains("no changed test files"));
    p.file("theme.json", "{\"changed\":true}");
    let changed = run(&p);
    assert!(changed.all().contains("a.test.js"), "{}", changed.all());
    assert!(!changed.all().contains("b.test.js"), "{}", changed.all());
}

#[test]
fn changed_runs_all_candidates_if_vite_is_unavailable() {
    let p = Project::new("vite-watch-git-fallback");
    p.with_esbuild();
    p.file(".gitignore", "node_modules/\n.tt-cache/\n.tt-tmp/\n");
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("theme.json", "{}");
    p.file("a.test.js", "test('a', () => {});");
    p.file("b.test.js", "test('b', () => {});");
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "base",
        ][..],
    ] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&p.dir)
            .status()
            .unwrap()
            .success());
    }
    p.file("theme.json", "{\"changed\":true}");
    let result = turbo_test(&p.dir, &["--changed", "--jobs", "1"], &[]);
    assert_eq!(result.code, Some(0), "{}", result.all());
    assert!(
        result.all().contains("a.test.js") && result.all().contains("b.test.js"),
        "{}",
        result.all()
    );
    assert!(
        result.all().contains("running all candidate tests"),
        "{}",
        result.all()
    );
}

#[test]
fn changes_to_forced_config_invalidate_previous_runs() {
    let p = Project::new("forced-config-history");
    p.with_esbuild();
    p.file(
        "custom.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file("a.test.js", "test('a', () => {});");
    let args = ["--since-last-run", "--config", "custom.config.js"];
    let first = turbo_test(&p.dir, &args, &[]);
    assert_eq!(first.code, Some(0), "{}", first.all());
    assert!(turbo_test(&p.dir, &args, &[])
        .all()
        .contains("no changed test files"));
    p.file(
        "custom.config.js",
        "export default { test: { environment: 'node' } }; // updated",
    );
    let changed = turbo_test(&p.dir, &args, &[]);
    assert!(changed.all().contains("a.test.js"), "{}", changed.all());
}

#[test]
fn vite_watch_hooks_on_imported_css_are_tracked() {
    let p = Project::new("vite-watch-css");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("dep.css", ".example { color: red; }");
    p.file("theme.json", "{}");
    p.file("a.test.js", "import './dep.css'; test('a', () => {});");
    p.file("b.test.js", "test('b', () => {});");
    let first = run(&p);
    assert_eq!(first.code, Some(0), "{}", first.all());
    assert!(run(&p).all().contains("no changed test files"));
    p.file("theme.json", "{\"changed\":true}");
    let changed = run(&p);
    assert!(changed.all().contains("a.test.js"), "{}", changed.all());
    assert!(!changed.all().contains("b.test.js"), "{}", changed.all());
}

#[test]
fn global_watch_stays_global_when_a_module_watches_the_same_file() {
    let p = Project::new("vite-overlapping-global-watch");
    p.with_esbuild();
    install_vite_graph_fixture(&p);
    p.file("vite.config.js", "export default { plugins: [{}] };");
    p.file("global-target", "");
    p.file("dep.js", "export const value = 1;");
    p.file("theme.json", "{}");
    p.file(
        "a.test.js",
        "import {value} from './dep.js'; test('a', () => expect(value).toBe(1));",
    );
    p.file("b.test.js", "test('b', () => {});");
    assert_eq!(run(&p).code, Some(0));
    assert!(run(&p).all().contains("no changed test files"));
    p.file("theme.json", "{\"changed\":true}");
    let changed = run(&p);
    assert!(
        changed.all().contains("a.test.js") && changed.all().contains("b.test.js"),
        "{}",
        changed.all()
    );
}
