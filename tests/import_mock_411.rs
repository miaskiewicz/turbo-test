//! vi.importMock must return mocks without mutating the actual module (#411).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn import_mock_generates_namespace_and_preserves_registered_mocks() {
    let p = Project::new("import-mock-411");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"import-mock-411","type":"module"}"#,
    );
    p.file(
        "vitest.config.js",
        "export default {test:{environment:'node'}};",
    );
    p.file("value.js", include_str!("data/import-mock-411/value.js"));
    p.file("manual.js", include_str!("data/import-mock-411/manual.js"));
    p.file(
        "__mocks__/manual.js",
        include_str!("data/import-mock-411/manual-mock.js"),
    );
    p.file("factory.js", "export const run=()=> 'unmocked-factory';");
    p.file(
        "throws.js",
        "throw new Error('import-mock evaluation failure');",
    );
    p.file(
        "proof.test.js",
        include_str!("data/import-mock-411/proof.test.js"),
    );
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
