//! Resolver state and configured mappings stay authoritative in opted-in Vue projects.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::ffi::OsString;
use std::process::Command;
use turbo_test::runner::resolve_spec_as;

struct RestoreEnvironment(Vec<(&'static str, Option<OsString>)>);
impl RestoreEnvironment {
    fn new(keys: &[&'static str]) -> Self {
        Self(
            keys.iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        )
    }
}
impl Drop for RestoreEnvironment {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn plain_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"vue-resolver-084"}"#);
    p
}
fn cache_mode_flips() {
    let p = plain_project("vue-resolver-mode-084");
    let widget = p.file("Widget.vue", "<template><p>widget</p></template>");
    let other = p.file("Other.vue", "<template><p>other</p></template>");
    std::env::set_var("TURBO_VUE", "0");
    assert_eq!(resolve_spec_as("./Widget", &p.dir, false), None);
    std::env::set_var("TURBO_VUE", "1");
    assert_eq!(resolve_spec_as("./Widget", &p.dir, false), Some(widget));
    assert_eq!(resolve_spec_as("./Other", &p.dir, false), Some(other));
    std::env::set_var("TURBO_VUE", "0");
    assert_eq!(resolve_spec_as("./Other", &p.dir, false), None);
}
fn dual_package(p: &Project) {
    p.file("node_modules/custom-vue/package.json", r#"{"name":"custom-vue","exports":{".":{"import":"./import.mjs","require":"./require.cjs"}}}"#);
    p.file(
        "node_modules/custom-vue/import.mjs",
        "export const kind = 'import';\n",
    );
    p.file(
        "node_modules/custom-vue/require.cjs",
        "exports.kind = 'require';\n",
    );
    p.file("require.test.cjs", "const { kind } = require('vue'); test('mapped require uses require conditions', () => expect(kind).toBe('require'));\n");
    p.file("import.test.mjs", "import { kind } from 'vue'; await Promise.resolve(); test('mapped import uses import conditions', () => expect(kind).toBe('import'));\n");
}
fn node_conditions(p: &Project) {
    let run = Command::new("node").args(["--input-type=module", "-e", "import assert from 'node:assert/strict'; import {createRequire} from 'node:module'; const require=createRequire(import.meta.url); assert.equal(require('custom-vue').kind,'require'); assert.equal((await import('custom-vue')).kind,'import');"])
        .current_dir(&p.dir).output().unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
fn passes(p: &Project, count: u64) {
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_VUE", "1")]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
fn dual_mappings() {
    let alias = plain_project("vue-resolver-dual-alias-084");
    dual_package(&alias);
    node_conditions(&alias);
    alias.file(
        "vitest.config.mjs",
        "export default {resolve:{alias:{vue:'custom-vue'}},test:{environment:'node'}};\n",
    );
    passes(&alias, 2);
    let mapper = plain_project("vue-resolver-dual-mapper-084");
    dual_package(&mapper);
    node_conditions(&mapper);
    mapper.file(
        "jest.config.cjs",
        "module.exports={testEnvironment:'jsdom',moduleNameMapper:{'^vue$':'custom-vue'}};\n",
    );
    passes(&mapper, 2);
}
fn configured_vue_paths() {
    let p = plain_project("vue-resolver-tsconfig-084");
    let custom = p.file("custom.js", "exports.kind = 'custom';\n");
    let widget = p.file(
        "components/Widget.vue",
        "<template><p>mapped</p></template>",
    );
    p.file("tsconfig.base.json", "{ // inherited mappings use the resolver's JSONC parser\n\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"vue\":[\"./custom.js\"],\"@components/*\":[\"./components/*\"]}}}\n");
    p.file("tsconfig.json", r#"{"extends":"./tsconfig.base.json"}"#);
    assert_eq!(resolve_spec_as("vue", &p.dir, false), Some(custom));
    assert_eq!(
        resolve_spec_as("@components/Widget", &p.dir, false),
        Some(widget)
    );
    let alias = plain_project("vue-resolver-component-alias-084");
    let component = alias.file(
        "components/Widget.vue",
        "<template><p>aliased</p></template>",
    );
    std::env::set_var(
        "TURBO_ALIAS",
        format!("@components\u{1f}{}", alias.path("components").display()),
    );
    let from = alias.dir.clone();
    let resolved = std::thread::spawn(move || resolve_spec_as("@components/Widget", &from, false))
        .join()
        .unwrap();
    assert_eq!(resolved, Some(component));
    std::env::remove_var("TURBO_ALIAS");
}
fn runtime_flags() {
    let p = plain_project("vue-resolver-context-env-084");
    let widget = p.file("Widget.vue", "<template><p>runtime</p></template>");
    let source = format!("test('current runtime Vue mode controls resolver caches',()=>{{ process.env.TURBO_VUE='0'; expect(()=>require.resolve('./Widget')).toThrow(); process.env.TURBO_VUE='1'; expect(require.resolve('./Widget')).toBe({widget:?}); process.env.TURBO_VUE='0'; expect(()=>require.resolve('./Widget')).toThrow(); }});\n");
    p.file("mode.test.cjs", &source);
    passes(&p, 1);
}

#[test]
fn vue_resolver_mode_and_mappings_are_preserved_in_one_batch() {
    let _restore = RestoreEnvironment::new(&["TURBO_VUE", "TURBO_ALIAS", "TURBO_NO_E12"]);
    std::env::remove_var("TURBO_ALIAS");
    std::env::remove_var("TURBO_NO_E12");
    cache_mode_flips();
    std::env::set_var("TURBO_VUE", "1");
    dual_mappings();
    configured_vue_paths();
    runtime_flags();
}
