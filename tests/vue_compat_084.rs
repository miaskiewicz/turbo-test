//! Opt-in Vue SFC fixtures, controlled by real @vitejs/plugin-vue + Vitest.
#![cfg(unix)]
mod common;
use common::{json_report, repo_root, turbo_test, Project};
use std::os::unix::fs::symlink;

fn project(name: &str, plugin: bool) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", if plugin {
        r#"{"name":"vue-084","type":"module","devDependencies":{"@vitejs/plugin-vue":"6.0.11"}}"#
    } else { r#"{"name":"vue-084","type":"module"}"# });
    p.file(
        "vitest.config.mjs",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    for name in ["vue", "@vue"] {
        symlink(
            repo_root().join("node_modules").join(name),
            p.path("node_modules").join(name),
        )
        .unwrap();
    }
    p
}

fn fixture(p: &Project) {
    for (name, source) in [
        ("Counter.vue", include_str!("data/vue/Counter.vue")),
        ("Options.vue", include_str!("data/vue/Options.vue")),
        (
            "TemplateOnly.vue",
            include_str!("data/vue/TemplateOnly.vue"),
        ),
        ("vue.test.js", include_str!("data/vue/vue.test.js")),
        (
            "require.test.cjs",
            include_str!("data/vue/require.test.cjs"),
        ),
        ("esm.test.mjs", include_str!("data/vue/esm.test.mjs")),
    ] {
        p.file(name, source);
    }
}

fn passes(p: &Project, env: &[(&str, &str)]) {
    let run = turbo_test(&p.dir, &["--reporter", "json"], env);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn vue_sfc_opt_in_loads_import_require_and_native_esm_graphs() {
    let p = project("vue-opt-in-084", false);
    fixture(&p);
    passes(&p, &[("TURBO_VUE", "1")]);
}

#[test]
fn declared_vue_plugin_enables_sfc_loading_without_changing_other_projects() {
    let p = project("vue-plugin-084", true);
    fixture(&p);
    passes(&p, &[]);
}

#[test]
fn vue_sfc_disabled_mode_reports_the_missing_opt_in() {
    let p = project("vue-disabled-084", true);
    p.file("Counter.vue", include_str!("data/vue/Counter.vue"));
    p.file("disabled.test.js", "import Counter from './Counter.vue'; test('not reached', () => expect(Counter).toBeDefined());\n");
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_VUE", "0")]);
    assert_ne!(run.code, Some(0), "{}", run.all());
    assert!(
        run.all().contains("Vue SFC support requires"),
        "{}",
        run.all()
    );
}

#[test]
fn malformed_vue_source_reports_the_real_compiler_error() {
    let p = project("vue-invalid-084", false);
    p.file(
        "Broken.vue",
        "<script setup lang=\"ts\">const invalid: = 1;</script><template><p>broken</p></template>",
    );
    p.file("broken.test.js", "import Broken from './Broken.vue'; test('not reached', () => expect(Broken).toBeDefined());\n");
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_VUE", "1")]);
    assert_ne!(run.code, Some(0), "{}", run.all());
    assert!(
        run.all().contains("Vue SFC compile failed"),
        "{}",
        run.all()
    );
    assert!(run.all().contains("Unexpected token"), "{}", run.all());
}
