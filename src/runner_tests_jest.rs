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
