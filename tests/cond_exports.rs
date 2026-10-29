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
