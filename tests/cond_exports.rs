//! Conditional `exports` resolution (turbo-test#200): an ES import, `import()` and
//! `import.meta.resolve` match a package's `exports` keys in the package's own order against the
//! import conditions (never `require`), and `require()` against the require conditions. The
//! project in `tests/data/cond-exports` holds `require`-before-`default`, `require`-before-
//! `import`, `default`-first, nested-condition and subpath / pattern exports; `cases/cond.test.ts`
//! passes under vitest 4.1.12 (environment node and happy-dom).
#![cfg(unix)]

mod common;
use common::{json_report, repo_root, turbo_test, Project};
use std::path::Path;

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), &dst).unwrap();
        }
    }
}

#[test]
fn conditional_exports_follow_key_order_like_vitest() {
    let data = repo_root().join("tests/data/cond-exports");
    for esbuild in [true, false] {
        for env in ["node", "happy-dom"] {
            let p = Project::new(&format!("cond-exports-{esbuild}-{env}"));
            p.file("package.json", "{\"type\":\"module\"}\n");
            copy_tree(&data.join("cases"), &p.path("cases"));
            copy_tree(&data.join("pkgs"), &p.path("node_modules"));
            if esbuild {
                p.with_esbuild();
            }
            p.file(
                "vitest.config.mjs",
                &format!("export default {{ test: {{ globals: true, environment: '{env}', include: ['cases/**/*.test.ts'] }} }}\n"),
            );
            let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
            let j = json_report(&r);
            assert_eq!(
                (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
                (Some(4), Some(0)),
                "esbuild={esbuild} env={env}\n{}",
                r.all()
            );
        }
    }
}

/// turbo-test#211: vitest resolves a package's `development` export condition (it runs in
/// development mode), so @emotion/sheet loads its development build and appends text nodes to
/// its `<style>` (ui-design-components' ArticleCard reads them). vitest 4.1.12: 1 passed under
/// node and jsdom; turbo-test took the production build.
#[test]
fn the_development_condition_is_matched_like_vitest() {
    for esbuild in [true, false] {
        let p = Project::new(&format!("cond-development-{esbuild}"));
        p.file("package.json", "{\"type\":\"module\"}\n");
        if esbuild {
            p.with_esbuild();
        }
        p.file(
            "node_modules/devpkg/package.json",
            r#"{"name":"devpkg","exports":{".":{"development":{"import":"./dev.mjs","default":"./dev.cjs"},"import":"./prod.mjs","default":"./prod.cjs"}}}"#,
        );
        for (f, v) in [
            ("dev.mjs", "export const build = 'development';\n"),
            ("prod.mjs", "export const build = 'production';\n"),
            ("dev.cjs", "exports.build = 'development-cjs';\n"),
            ("prod.cjs", "exports.build = 'production-cjs';\n"),
        ] {
            p.file(&format!("node_modules/devpkg/{f}"), v);
        }
        p.file(
            "a.test.ts",
            "import { expect, it } from 'vitest';\nimport { build } from 'devpkg';\nimport { createRequire } from 'node:module';\n\
             it('dev', () => { expect([build, createRequire(import.meta.url)('devpkg').build]).toEqual(['development', 'development-cjs']); });\n",
        );
        let r = turbo_test(&p.dir, &[], &[]);
        assert!(
            r.stdout.contains("Tests  1 passed (1)"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}

/// turbo-test#213: two installs of a package whose entry file is identical but whose inner
/// modules differ (user-event 14.6.1 vs 14.6.8) each load their own bundle from a shared cache.
#[test]
fn a_package_bundle_is_cached_per_install() {
    let cache = Project::new("dep-cache-shared");
    for (name, inner) in [("dep-cache-a", "old"), ("dep-cache-b", "new")] {
        let p = Project::new(name);
        p.with_esbuild();
        p.file("package.json", "{\"type\":\"module\"}\n");
        p.file(
            "node_modules/samepkg/package.json",
            &format!("{{\"name\":\"samepkg\",\"version\":\"1.0.{}\",\"exports\":{{\".\":{{\"require\":\"./cjs.js\",\"default\":\"./index.js\"}}}}}}", inner.len()),
        );
        p.file(
            "node_modules/samepkg/index.js",
            "export { v } from './inner.js';\n",
        );
        p.file(
            "node_modules/samepkg/inner.js",
            &format!("export const v = '{inner}';\n"),
        );
        p.file(
            "a.test.ts",
            &format!("import {{ expect, it }} from 'vitest';\nimport {{ v }} from 'samepkg';\nit('own', () => {{ expect(v).toBe('{inner}'); }});\n"),
        );
        let r = turbo_test(
            &p.dir,
            &[],
            &[("TURBO_CACHE_DIR", cache.dir.to_str().unwrap())],
        );
        assert!(
            r.stdout.contains("Tests  1 passed (1)"),
            "{name}\n{}",
            r.all()
        );
    }
}
