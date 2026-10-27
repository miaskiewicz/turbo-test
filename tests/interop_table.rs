//! Default/named-export interop table (turbo-test#122): the project in `tests/data/interop122`
//! imports CommonJS and ES modules every way a React/TS suite does — `module.exports = X`,
//! `exports.default` with and without `__esModule`, tsc and babel output (`__importDefault`,
//! `__importStar`, `_interopRequireDefault`, `_interopRequireWildcard`), re-exports of a default
//! (`export { default } from`, `export * from` a CommonJS module), packages with an `exports` map
//! or a `module` field, `vi.mock` / `vi.importActual` of CommonJS, a CommonJS package that
//! requires an ESM-only one, `import()` and `createRequire`. Each row describes the value it got
//! (`cases/d.ts`) and compares it with vitest 4.1.12's description (`cases/expected.ts`), which is
//! the same under `environment: 'happy-dom'` and `'node'`, `isolate` true and false.
//!
//! The rows run on both of turbo-test's loaders: the module runner (a project with
//! `node_modules/.bin/esbuild`) and the ES-module graph loader (a project without it, as a vite 8
//! install has none).
#![cfg(unix)]

mod common;
use common::{json_report, repo_root, turbo_test, Project};
use std::path::Path;

/// Rows turbo-test still answers differently from vitest, with the issue that tracks each and
/// turbo-test's description: `(loader, row, turbo-test's value, issue)`. `loader` is `"mr"`,
/// `"graph"` or `"*"` (both).
const KNOWN: &[(&str, &str, &str, &str)] = &[];

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

fn project(name: &str, esbuild: bool) -> Project {
    let data = repo_root().join("tests/data/interop122");
    let p = Project::new(name);
    p.file("package.json", "{\"type\":\"module\"}\n");
    copy_tree(&data.join("cases"), &p.path("cases"));
    copy_tree(&data.join("src"), &p.path("src"));
    copy_tree(&data.join("pkgs"), &p.path("node_modules"));
    if esbuild {
        p.with_esbuild();
    }
    p
}

/// Run the table under every vitest config; returns `config: row: message` for each row that
/// failed (vitest's description differs) and is not a KNOWN row of `loader`.
fn run_table(p: &Project, loader: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for env in ["happy-dom", "node"] {
        for isolate in [true, false] {
            p.file(
                "vitest.config.mjs",
                &format!(
                    "export default {{ test: {{ environment: '{env}', isolate: {isolate}, include: ['cases/**/*.test.ts'] }} }}\n"
                ),
            );
            // one worker under isolate: false, so the files share their module registry
            let jobs = if isolate { "2" } else { "1" };
            let r = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_JOBS", jobs)]);
            let report = json_report(&r);
            let files = report["testResults"].as_array().unwrap();
            assert_eq!(files.len(), 4, "{}", r.all());
            let mut rows = 0;
            for f in files {
                let results = f["assertionResults"].as_array().unwrap();
                assert!(!results.is_empty(), "{f}");
                for a in results {
                    rows += 1;
                    let title = a["title"].as_str().unwrap();
                    let file = Path::new(f["name"].as_str().unwrap())
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .replace(".test.ts", "");
                    let row = format!("{file}|{title}");
                    let msg = a["failureMessages"]
                        .as_array()
                        .and_then(|m| m.first())
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let known = KNOWN
                        .iter()
                        .find(|k| (k.0 == loader || k.0 == "*") && k.1 == row);
                    match (a["status"].as_str().unwrap(), known) {
                        ("passed", None) => {}
                        ("passed", Some(k)) => bad.push(format!(
                            "{env} isolate={isolate}: {row}: passes now, drop it from KNOWN ({})",
                            k.3
                        )),
                        (_, Some(k)) if msg.contains(&format!("GOT<{}>: WANT<", k.2)) => {}
                        _ => bad.push(format!("{env} isolate={isolate}: {row}: {msg}")),
                    }
                }
            }
            assert_eq!(rows, 89, "{}", r.all());
        }
    }
    bad
}

#[test]
fn interop_table_matches_vitest_on_the_module_runner() {
    let p = project("interop-table-mr", true);
    let bad = run_table(&p, "mr");
    assert!(
        bad.is_empty(),
        "{} rows differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn interop_table_matches_vitest_on_the_esm_graph_loader() {
    let p = project("interop-table-graph", false);
    let bad = run_table(&p, "graph");
    assert!(
        bad.is_empty(),
        "{} rows differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
