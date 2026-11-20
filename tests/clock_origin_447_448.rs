//! The default clock reads the real wall clock at each run (#447, #448). The framework snapshot is
//! cached per build, so the second run below deserializes a snapshot made by the first; a clock
//! origin captured at snapshot creation would be that pause behind. The data file also passes
//! under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn real_now_ms() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    now.as_millis().to_string()
}

#[test]
fn the_default_clock_reads_the_host_clock_on_a_cached_snapshot() {
    let p = Project::new("clock-origin-447");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"clock-origin-447"}"#);
    p.file(
        "clock.test.cjs",
        include_str!("data/compat-085/clock-origin.cjs"),
    );
    let cache = p.path(".turbo-cache");
    let cache = cache.to_str().unwrap();
    let first = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CACHE_DIR", cache), ("REAL_NOW", &real_now_ms())],
    );
    assert_eq!(json_report(&first)["numTotalTests"], 3, "{}", first.all());
    std::thread::sleep(Duration::from_millis(2500));
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_CACHE_DIR", cache), ("REAL_NOW", &real_now_ms())],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
