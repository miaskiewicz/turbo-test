//! `globalThis.__ttNodeFeatures` exposes the active Node-feature-set version (22/24/26) to the
//! runtime so code paths can switch per run (turbo-test#453). Precedence: CLI > env > config >
//! default 22. Read at site-of-use (install_runtime_flags), NOT snapshot time, so a cached
//! framework snapshot never ships a stale value.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const TEST_FILE: &str = r#"
const { it, expect } = require('vitest');
it('exposes __ttNodeFeatures', () => {
  expect(typeof globalThis.__ttNodeFeatures).toBe('number');
  expect(globalThis.__ttNodeFeatures).toBe(Number(process.env.EXPECT));
});
"#;

fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"nf-453"}"#);
    p.file("nf.test.cjs", TEST_FILE);
    p
}

#[test]
fn default_is_22() {
    let p = project("nf-default");
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[("EXPECT", "22")]);
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn cli_flag_sets_24() {
    let p = project("nf-cli-24");
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--node-features", "24"],
        &[("EXPECT", "24")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn env_sets_26() {
    let p = project("nf-env-26");
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("EXPECT", "26"), ("TURBO_NODE_FEATURES", "26")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn config_sets_24() {
    let p = project("nf-config-24");
    p.file(
        "vitest.config.ts",
        "export default { test: { nodeFeatures: 24 } };",
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[("EXPECT", "24")]);
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn cli_beats_env_and_config() {
    let p = project("nf-precedence");
    p.file(
        "vitest.config.ts",
        "export default { test: { nodeFeatures: 26 } };",
    );
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--node-features", "22"],
        &[("EXPECT", "22"), ("TURBO_NODE_FEATURES", "24")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn env_beats_config() {
    let p = project("nf-env-beats-config");
    p.file(
        "vitest.config.ts",
        "export default { test: { nodeFeatures: 26 } };",
    );
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("EXPECT", "22"), ("TURBO_NODE_FEATURES", "22")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 1, "{}", run.all());
}
