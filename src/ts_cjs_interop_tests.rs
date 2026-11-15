use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "turbo-ts-interop-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
#[test]
fn jsonc_inheritance_uses_last_parent_then_child_and_tracks_edits() {
    let dir = directory();
    let config = dir.join("tsconfig.json");
    std::fs::write(dir.join("base.json"), r#"{"compilerOptions":{"module":"CommonJS","esModuleInterop":false},"url":"https://example.invalid"}"#).unwrap();
    std::fs::write(
        &config,
        "{ /* options from base */ \"extends\":\"./base.json\", }",
    )
    .unwrap();
    assert!(enabled(&config));
    std::fs::write(
        dir.join("override.json"),
        r#"{"compilerOptions":{"esModuleInterop":true}}"#,
    )
    .unwrap();
    std::fs::write(&config, r#"{"extends":["./base.json","./override.json"]}"#).unwrap();
    assert!(!enabled(&config));
    std::fs::write(&config,r#"{"extends":["./base.json","./override.json"],"compilerOptions":{"esModuleInterop":false}}"#).unwrap();
    assert!(enabled(&config));
    std::fs::write(
        &config,
        r#"{"extends":"./base.json","compilerOptions":{"module":"ESNext"}}"#,
    )
    .unwrap();
    assert!(!enabled(&config));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn invalid_missing_cyclic_and_unconfigured_inputs_do_not_enable_a_policy() {
    let dir = directory();
    let config = dir.join("tsconfig.json");
    assert!(!enabled(&config));
    for text in [
        "not json",
        "{}",
        r#"{"compilerOptions":{"module":"commonjs"}}"#,
        r#"{"extends":"./missing.json"}"#,
        r#"{"extends":"./tsconfig.json","compilerOptions":{"module":"commonjs","esModuleInterop":false}}"#,
    ] {
        std::fs::write(&config, text).unwrap();
        assert!(!enabled(&config));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
