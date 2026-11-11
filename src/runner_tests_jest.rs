//! Unit tests for runner.rs's jest-config runtime literal (turbo-test#186).

use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "turbo-test-unit-jest-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// A jest config's fakeTimers block reaches the runtime as `jestFakeTimers`, beside the
// zeroDelayTimers env switch; a jest config without one gives only the env's part.
#[test]
fn a_jest_configs_fake_timers_reach_the_runtime() {
    let d = dir("ft");
    std::fs::write(
        d.join("jest.config.js"),
        "module.exports = { fakeTimers: { enableGlobally: true, now: 5 } }\n",
    )
    .unwrap();
    assert_eq!(
        runtime_config_for(&d, None).as_deref(),
        Some("{ jestFakeTimers: { enableGlobally: true, now: 5 } }")
    );
    assert_eq!(
        runtime_config_for(&d, Some("0")).as_deref(),
        Some("{ jestFakeTimers: { enableGlobally: true, now: 5 }, zeroDelayTimers: false }")
    );
    std::fs::write(d.join("jest.config.js"), "module.exports = {}\n").unwrap();
    assert_eq!(runtime_config_for(&d, None), None);
    assert_eq!(
        runtime_config_for(&d, Some("1")).as_deref(),
        Some("{ zeroDelayTimers: true }")
    );
    // a package.json "jest" block is a jest config too
    let p = dir("pkg");
    std::fs::write(
        p.join("package.json"),
        r#"{ "jest": { "fakeTimers": { "enableGlobally": true } } }"#,
    )
    .unwrap();
    assert_eq!(
        runtime_config_for(&p, None).as_deref(),
        Some(r#"{ jestFakeTimers: {"enableGlobally":true} }"#)
    );
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&p);
}

// A jest config's testTimeout reaches the runtime as `jestTestTimeout` (turbo-test#346).
#[test]
fn a_jest_configs_test_timeout_reaches_the_runtime() {
    let d = dir("timeout");
    std::fs::write(
        d.join("jest.config.js"),
        "module.exports = { testTimeout: 35 }\n",
    )
    .unwrap();
    let cfg = runtime_config_for(&d, None);
    assert!(
        cfg.as_deref()
            .unwrap_or("")
            .contains("jestTestTimeout: 35"),
        "expected jestTestTimeout: 35, got {cfg:?}"
    );
    // a non-numeric testTimeout is ignored
    std::fs::write(
        d.join("jest.config.js"),
        "module.exports = { testTimeout: 'slow' }\n",
    )
    .unwrap();
    assert_eq!(runtime_config_for(&d, None), None);
    let _ = std::fs::remove_dir_all(&d);
}

// Jest setupFiles vs setupFilesAfterEnv split (turbo-test#344).
#[test]
fn jest_setup_files_split_early_and_after_env() {
    let d = dir("setup-split");
    std::fs::write(d.join("early.js"), "// early\n").unwrap();
    std::fs::write(d.join("late.js"), "// late\n").unwrap();
    std::fs::write(
        d.join("jest.config.js"),
        "module.exports = { setupFiles: ['./early.js'], setupFilesAfterEnv: ['./late.js'] };\n",
    )
    .unwrap();
    let found = jest_setup_files(&d).unwrap();
    assert_eq!(found.len(), 1, "after-env should have 1 file: {found:?}");
    assert!(
        found[0].ends_with("late.js"),
        "after-env should be late.js: {found:?}"
    );
    let early: Vec<PathBuf> = JEST_EARLY_SETUP.with(|f| f.borrow().clone());
    assert_eq!(early.len(), 1, "early should have 1 file: {early:?}");
    assert!(
        early[0].ends_with("early.js"),
        "early should be early.js: {early:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// Jest clearMocks / resetMocks / restoreMocks config (turbo-test#343).
#[test]
fn jest_config_clear_reset_restore_mocks() {
    let d = dir("mocks-cfg");
    std::fs::write(
        d.join("jest.config.js"),
        "module.exports = { clearMocks: true, resetMocks: true, restoreMocks: true }\n",
    )
    .unwrap();
    let cfg = runtime_config_for(&d, None).unwrap();
    assert!(cfg.contains("clearMocks: true"), "clearMocks missing: {cfg}");
    assert!(
        cfg.contains("mockReset: true"),
        "mockReset missing: {cfg}"
    );
    assert!(
        cfg.contains("restoreMocks: true"),
        "restoreMocks missing: {cfg}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
