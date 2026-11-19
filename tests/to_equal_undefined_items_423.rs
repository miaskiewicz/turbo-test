//! toEqual and undefined array items (turbo-test#423). Jest's `equals` compares array lengths only
//! under toStrictEqual, so `['a', 'b', undefined]` toEqual `['a', 'b']` passes in Jest (an
//! undefined item is an undefined property); vitest's checks the length always. Each project kind
//! keeps its own runner's semantics. The rows are what Jest 30.5.2 and vitest 5.1.0 print for
//! tests/data/compat-085/undefined-items-matrix.js (columns: toEqual, toStrictEqual,
//! objectContaining, toHaveBeenCalledWith, toMatchObject, arrayContaining).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const MATRIX: &str = include_str!("data/compat-085/undefined-items-matrix.js");

const JEST_ROWS: &str = r#"[["trailing undefined","pass","fail","pass","pass","pass","pass"],["expected trailing undefined","pass","fail","pass","pass","pass","pass"],["undefined vs hole","pass","fail","pass","pass","pass","pass"],["undefined property","pass","fail","pass","pass","pass","pass"],["nested array","pass","fail","pass","pass","pass","pass"],["undefined vs null item","fail","fail","fail","fail","fail","fail"],["empty vs undefined","pass","fail","pass","pass","pass","pass"],["two trailing","pass","fail","pass","pass","pass","pass"],["undefined then value","fail","fail","fail","fail","fail","fail"],["array own undefined property","pass","fail","pass","pass","pass","pass"]]"#;

const VITEST_ROWS: &str = r#"[["trailing undefined","fail","fail","fail","fail","fail","fail"],["expected trailing undefined","fail","fail","fail","fail","fail","fail"],["undefined vs hole","pass","fail","pass","pass","pass","pass"],["undefined property","pass","fail","pass","pass","pass","pass"],["nested array","fail","fail","fail","fail","fail","fail"],["undefined vs null item","fail","fail","fail","fail","fail","fail"],["empty vs undefined","fail","fail","fail","fail","fail","fail"],["two trailing","fail","fail","fail","fail","fail","fail"],["undefined then value","fail","fail","fail","fail","fail","fail"],["array own undefined property","pass","fail","pass","pass","pass","pass"]]"#;

fn run_matrix(p: &Project, rows: &str) {
    p.file(
        "matrix.test.js",
        &format!("globalThis.EXPECTED = {rows};\n{MATRIX}"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn jest_to_equal_ignores_undefined_array_items() {
    let p = Project::new("to-equal-undefined-items-423-jest");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };",
    );
    run_matrix(&p, JEST_ROWS);
}

#[test]
fn vitest_to_equal_keeps_array_length_strict() {
    let p = Project::new("to-equal-undefined-items-423-vitest");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file(
        "vitest.config.mjs",
        "export default { test: { globals: true } };",
    );
    run_matrix(&p, VITEST_ROWS);
}
