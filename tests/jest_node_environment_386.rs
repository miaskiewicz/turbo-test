#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn jest_node_config_keeps_package_browser_shim_in_node_branch() {
    let project = Project::new("jest-node-environment-386");
    project.with_esbuild();
    project.file("package.json", r#"{"name":"jest-node-environment-386"}"#);
    project.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    project.file(
        "node_modules/tsup-shim/package.json",
        r#"{"name":"tsup-shim","main":"index.cjs"}"#,
    );
    project.file("node_modules/tsup-shim/index.cjs", r#"module.exports = typeof document === 'undefined' ? new URL(`file:${__filename}`).href : new URL('main.js', document.baseURI).href;"#);
    project.file("environment.spec.ts", r#"const url = require('tsup-shim'); test('document is absent in node',()=>{expect(typeof document).toBe('undefined');expect(url.startsWith('file:')).toBe(true);});"#);
    let run = turbo_test(&project.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
