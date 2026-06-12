//! M5 — affected-test detection CLI (watch-mode core).
//!
//! Usage: m5-affected --changed <file>[,<file>...] <test files...>
//! Prints the test files affected by the change (superset of truly-affected), plus the
//! over-selection ratio and analysis latency.

use std::path::PathBuf;
use std::time::Instant;

use turbo_test::graph::affected_tests;

fn main() {
    let mut changed: Vec<PathBuf> = Vec::new();
    let mut tests: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--changed" {
            if let Some(v) = args.next() {
                changed.extend(v.split(',').map(PathBuf::from));
            }
        } else {
            tests.push(PathBuf::from(a));
        }
    }
    if changed.is_empty() || tests.is_empty() {
        eprintln!("usage: m5-affected --changed <file>[,<file>] <test files...>");
        std::process::exit(2);
    }

    let t = Instant::now();
    let affected = affected_tests(&changed, &tests);
    let us = t.elapsed().as_secs_f64() * 1_000_000.0;

    println!("changed: {}", changed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    println!("affected test files ({} of {}):", affected.len(), tests.len());
    for a in &affected {
        println!("  {}", a.display());
    }
    let pct = 100.0 * affected.len() as f64 / tests.len() as f64;
    println!(
        "\nover-selection: {:.0}% of suite selected | analysis {:.0} us",
        pct, us
    );
}
