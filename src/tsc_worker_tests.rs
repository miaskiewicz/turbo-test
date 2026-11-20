//! The persistent TS worker keeps a single `node` + `require("typescript")` alive
//! across many files, so a spec with hundreds of decorated models stops paying the
//! ~300 ms cold `require("typescript")` cost per file (turbo-test#431).

#![cfg(unix)]

use super::{spawn_count, transform};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ts_dir() -> PathBuf {
    repo_root().join("node_modules/typescript")
}

const DECORATED: &str = r#"
function Tag<T extends Function>(ctor: T): T { return ctor; }
@Tag
export class Service {
  value = 1;
}
"#;

#[test]
fn transform_lowers_legacy_decorators_through_the_persistent_worker() {
    let root = repo_root();
    let out = transform(
        &root,
        &ts_dir(),
        &root.join("service.ts"),
        "commonjs",
        DECORATED,
    )
    .expect("worker transform returned None");
    assert!(
        out.contains("__decorate"),
        "decorator lowering missing: {out}"
    );
    assert!(
        out.contains("Reflect.metadata")
            || out.contains("__metadata")
            || out.contains("__decorate"),
        "decorator / metadata helper missing: {out}"
    );
    assert!(
        out.contains("exports.Service"),
        "commonjs export missing: {out}"
    );
}

#[test]
fn many_files_reuse_a_single_worker_subprocess() {
    let root = repo_root();
    let ts_dir = ts_dir();
    let before = spawn_count();
    for i in 0..8 {
        let src = format!(
            "function Tag<T extends Function>(ctor: T): T {{ return ctor; }} @Tag export class K{i} {{ v = {i}; }}\n"
        );
        let file = root.join(format!("k{i}.ts"));
        let code = transform(&root, &ts_dir, &file, "commonjs", &src)
            .unwrap_or_else(|| panic!("worker transform returned None for k{i}"));
        assert!(code.contains(&format!("exports.K{i}")));
        assert!(code.contains("__decorate"));
    }
    let spawned = spawn_count() - before;
    assert!(
        spawned <= 1,
        "the persistent worker should serve many files from one node subprocess; spawned {spawned} times"
    );
}
