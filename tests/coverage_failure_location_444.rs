//! Under `--coverage` a failing assertion's reported line:column must match the position the
//! same spec reports without `--coverage` (turbo-test#444). Pre-existing: the esbuild path (used
//! for every file under coverage) emitted an inline source map but never fed it to the failure
//! frame mapping, so a failure pointed at the esbuild output's line/column instead of the .ts
//! source's (e.g. 5:36 instead of 4:17).
#![cfg(unix)]
mod common;
use common::{turbo_test, Project};

/// A spec with no mocks whose assertion fails on a known line/column.
const TS: &str = "\
import { expect, test } from 'vitest';
test('fails', () => {
  const value = 123;
  expect(value).toBe(456);
});
";

/// Grab the first `pkg.test.ts:<line>:<col>` frame location from the full run output.
fn frame_line(r: &common::Run) -> String {
    let out = format!("{}\n{}", r.stdout, r.stderr);
    let needle = "pkg.test.ts:";
    let start = out
        .match_indices(needle)
        .find(|(i, _)| out[i + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_else(|| panic!("no frame line in:\n{out}"))
        .0;
    let rest = &out[start..];
    let end = rest
        .find(|c: char| !matches!(c, 'p'..='z' | 'a'..='o' | '.' | ':' | '0'..='9'))
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn coverage_reports_same_failure_position_as_without_coverage() {
    let p = Project::new("cov-fail-444");
    p.with_esbuild();
    p.file("pkg.test.ts", TS);

    let env = &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")];
    let plain = turbo_test(&p.dir, &[], env);
    let covered = turbo_test(&p.dir, &["--coverage"], env);
    assert_eq!(plain.code, Some(1), "{}", plain.all());
    assert_eq!(covered.code, Some(1), "{}", covered.all());

    let plain_at = frame_line(&plain);
    let covered_at = frame_line(&covered);
    assert_eq!(
        plain_at,
        "pkg.test.ts:4:17",
        "no-coverage oracle: {}",
        plain.all()
    );
    assert_eq!(
        covered_at,
        plain_at,
        "coverage frame drifted from the no-coverage source position:\n{}",
        covered.all()
    );
}
