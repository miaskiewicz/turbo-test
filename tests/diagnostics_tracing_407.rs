//! node:diagnostics_channel tracing channels and bound stores, as real Pino loads them (#407).
//! Expectations come from real Node v26.11.0; the data files also run under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn node_project(name: &str, file: &str, source: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(file, source);
    p
}

#[test]
fn tracing_channels_publish_lifecycle_context_and_stores_like_node() {
    let p = node_project(
        "diagnostics-tracing-407",
        "tracing.test.cjs",
        include_str!("data/compat-085/diagnostics-tracing.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 14, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn throwing_subscribers_and_transforms_surface_as_uncaught_without_breaking_publishers() {
    let p = node_project(
        "diagnostics-isolation-407",
        "isolation.test.cjs",
        include_str!("data/compat-085/diagnostics-isolation.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    let all = run.all();
    assert!(all.contains("uncaught subscriber-boom"), "{all}");
    assert!(all.contains("uncaught transform-boom"), "{all}");
}
