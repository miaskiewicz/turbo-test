//! Unit tests for the M0 spike's pure pieces (the subprocess run lives in tests/spikes.rs).

use super::*;

#[test]
fn gate_passes_at_exactly_5x_and_fails_below() {
    let (code, text) = gate_verdict(5.0);
    assert_eq!(code, 0);
    assert!(
        text.starts_with("\n==> M0 PASS — snapshot fresh-context is >5x cheaper"),
        "{text}"
    );
    assert!(
        text.ends_with("Premise validated. Proceed to M1."),
        "{text}"
    );
    assert_eq!(gate_verdict(110.0).0, 0);

    let (code, text) = gate_verdict(4.99);
    assert_eq!(code, 1);
    assert_eq!(
        text,
        "\n==> M0 FAIL — snapshot not >5x vs stock vitest env. Reconsider."
    );
    assert_eq!(
        gate_verdict(f64::NAN).0,
        1,
        "an unmeasurable speedup does not pass"
    );
}

#[test]
fn median_picks_the_middle_sample() {
    assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
    assert_eq!(
        median(vec![4.0, 1.0, 3.0, 2.0]),
        3.0,
        "upper median for even counts"
    );
    assert_eq!(median(vec![7.5]), 7.5);
}

#[test]
fn framework_source_scales_with_weight() {
    let empty = build_framework_src(0);
    assert!(empty.contains("globalThis.__turbo = (() => {"), "{empty}");
    assert!(
        empty.contains("globalThis.expect = globalThis.__turbo.expect;"),
        "{empty}"
    );
    assert!(!empty.contains("M.m0 ="), "{empty}");
    let three = build_framework_src(3);
    for i in 0..3 {
        assert!(
            three.contains(&format!("  M.m{i} = (a, b) => {{")),
            "{three}"
        );
    }
    assert!(!three.contains("M.m3 ="), "{three}");
}
