//! Configured TypeScript CommonJS namespace imports are not always ES-module namespaces.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn fixture(interop: bool, inherited: bool) -> Project {
    let p = Project::new("ts-cjs-namespace-384");
    p.with_esbuild().with_typescript();
    p.file("package.json", r#"{"name":"ts-cjs-namespace-384"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    let options = format!(
        r#"{{"compilerOptions":{{"module":"commonjs","esModuleInterop":{interop},"allowSyntheticDefaultImports":true,"target":"ES2021"}}}}"#
    );
    if inherited {
        p.file("base.json", &options);
        p.file(
            "tsconfig.json",
            "{\n// inherited compiler options\n\"extends\":\"./base.json\",\n}",
        );
    } else {
        p.file("tsconfig.json", &options);
    }
    p.file("expected.json", &format!(r#"{{"interop":{interop}}}"#));
    p.file(
        "node_modules/callable-package/package.json",
        r#"{"name":"callable-package","main":"index.cjs"}"#,
    );
    p.file(
        "node_modules/callable-package/index.cjs",
        include_str!("data/ts-cjs-interop/callable.cjs"),
    );
    p.file(
        "check.test.ts",
        include_str!("data/ts-cjs-interop/check.test.ts"),
    );
    p.file("cycle-a.ts", include_str!("data/ts-cjs-interop/cycle-a.ts"));
    p.file("cycle-b.ts", include_str!("data/ts-cjs-interop/cycle-b.ts"));
    p.file(
        "node-control.cjs",
        include_str!("data/ts-cjs-interop/node-control.cjs"),
    );
    p
}
#[test]
fn namespace_imports_match_project_typescript_options_and_cycles() {
    for (interop, inherited) in [(false, false), (true, false), (false, true)] {
        let p = fixture(interop, inherited);
        let node = std::process::Command::new("node")
            .args(["--test", "node-control.cjs"])
            .current_dir(&p.dir)
            .output()
            .unwrap();
        assert!(
            node.status.success(),
            "{} {}",
            String::from_utf8_lossy(&node.stdout),
            String::from_utf8_lossy(&node.stderr)
        );
        let run = turbo_test(
            &p.dir,
            &["--jobs", "1", "--reporter", "json", "check.test.ts"],
            &[],
        );
        assert_eq!(json_report(&run)["numPassedTests"], 2, "{}", run.all());
        assert_eq!(json_report(&run)["numFailedTests"], 0, "{}", run.all());
    }
}
