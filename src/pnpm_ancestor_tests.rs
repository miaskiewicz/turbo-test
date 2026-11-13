//! Opt-in pnpm hoist ancestry, without guessing an arbitrary store version.
use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("turbo-pnpm-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn directory(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn only_explicit_one_enables_the_fallback() {
    assert!(enabled_value(Some(std::ffi::OsStr::new("1"))));
    for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
        assert!(!enabled_value(value.map(std::ffi::OsStr::new)));
    }
}

#[test]
fn disabled_and_non_bare_requests_do_not_probe_the_store() {
    let fixture = Fixture::new();
    fixture.directory("node_modules/.pnpm/node_modules/pkg");
    let calls = Cell::new(0);
    let probe = |_| {
        calls.set(calls.get() + 1);
        Some(PathBuf::from("unexpected"))
    };
    assert_eq!(resolve("pkg", &fixture.0, false, probe), None);
    for spec in [
        "./pkg",
        "../pkg",
        "/pkg",
        "node:fs",
        "C:\\pkg",
        "#alias",
        "@scope",
        "@scope/../pkg",
    ] {
        assert_eq!(resolve(spec, &fixture.0, true, probe), None, "{spec}");
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn nearest_real_hoist_link_controls_the_selected_version_and_subpath() {
    let fixture = Fixture::new();
    fixture.directory("node_modules/.pnpm/node_modules/pkg");
    fixture.directory("workspace/node_modules/.pnpm/node_modules/pkg");
    let from = fixture.directory("workspace/tests/nested");
    let chosen = fixture.0.join("workspace/node_modules/.pnpm");
    assert_eq!(
        resolve("pkg/subpath", &from, true, |root| Some(root.to_path_buf())),
        Some(chosen)
    );
}

#[test]
fn installed_package_and_unresolved_nearest_hoist_do_not_select_another_version() {
    let fixture = Fixture::new();
    fixture.directory("node_modules/.pnpm/node_modules/pkg");
    fixture.directory("workspace/node_modules/.pnpm/node_modules/pkg");
    let from = fixture.directory("workspace/tests");
    let calls = Cell::new(0);
    assert_eq!(
        resolve("pkg", &from, true, |_| {
            calls.set(calls.get() + 1);
            None
        }),
        None
    );
    assert_eq!(calls.get(), 1);
    fixture.directory("workspace/node_modules/pkg");
    calls.set(0);
    assert_eq!(
        resolve("pkg/private", &from, true, |_| {
            calls.set(calls.get() + 1);
            Some(PathBuf::from("unexpected"))
        }),
        None
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn scoped_links_are_supported_but_unlinked_store_versions_are_not_scanned() {
    let fixture = Fixture::new();
    fixture.directory("node_modules/.pnpm/unlinked@9/node_modules/unlinked");
    fixture.directory("node_modules/.pnpm/node_modules/@scope/pkg");
    assert_eq!(
        resolve("unlinked", &fixture.0, true, |_| Some(PathBuf::from(
            "unexpected"
        ))),
        None
    );
    let store = fixture.0.join("node_modules/.pnpm");
    assert_eq!(
        resolve("@scope/pkg/subpath", &fixture.0, true, |root| Some(
            root.to_path_buf()
        )),
        Some(store)
    );
}
