//! A cache directory the user sets (TURBO_CACHE_DIR, e.g. one a CI job persists) is shared by
//! every turbo-test build that runs against it: an artifact another build wrote (a transformed
//! module, a bundle, a startup snapshot of its runtime) must never be served to this one, or an
//! upgrade keeps running the old runtime. The default temp cache is already per build
//! (turbo-test#207); this is the same for a custom one.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};
use std::path::Path;

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
    )
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// This build's tag (`<version>-<build id>`), the name its cache entries live under.
fn this_build() -> String {
    format!("{}-{}", env!("CARGO_PKG_VERSION"), env!("TT_BUILD_ID"))
}

#[test]
fn a_custom_cache_dir_never_serves_another_builds_artifacts() {
    let p = Project::new("cache-build-scope");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "cachebuild" }"#);
    p.file(
        "value.test.ts",
        "const VALUE: number = 1;\ntest('value', () => { expect(VALUE).toBe(1); });\n",
    );
    let cache = p.dir.join("ci-cache");
    let env = [("TURBO_CACHE_DIR", cache.to_str().unwrap())];
    let r = turbo_test(&p.dir, &["--reporter", "json"], &env);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());

    // Make what this run cached look like another build's: its transform of the test now says
    // VALUE = 2, and whatever it filed under this build's name moves to another build's name.
    let mut tampered = 0;
    for f in files(&cache) {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        if text.contains("VALUE = 1") {
            std::fs::write(&f, text.replace("VALUE = 1", "VALUE = 2")).unwrap();
            tampered += 1;
        }
    }
    assert!(tampered > 0, "the run cached no transform of the test");
    let ours = cache.join(this_build());
    if ours.is_dir() {
        std::fs::rename(&ours, cache.join("0.0.0-0000000000000000")).unwrap();
    }

    let r = turbo_test(&p.dir, &["--reporter", "json"], &env);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
    assert!(ours.is_dir(), "this build caches under {}", ours.display());
}
