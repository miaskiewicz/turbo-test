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

#[test]
fn changing_interop_mode_invalidates_the_same_source_cache_in_both_directions() {
    let p = fixture(false, false);
    for interop in [false, true, false] {
        p.file(
            "tsconfig.json",
            &format!(
                r#"{{"compilerOptions":{{"module":"commonjs","esModuleInterop":{interop}}}}}"#
            ),
        );
        p.file("expected.json", &format!(r#"{{"interop":{interop}}}"#));
        let run = turbo_test(
            &p.dir,
            &["--jobs", "1", "--reporter", "json", "check.test.ts"],
            &[],
        );
        assert_eq!(json_report(&run)["numPassedTests"], 2, "{}", run.all());
    }
}

#[test]
fn dynamic_imports_and_vitest_namespaces_keep_their_esm_contract() {
    let p = fixture(false, false);
    let oracle = std::process::Command::new("node").args(["--input-type=module", "-e", "import * as ns from 'callable-package'; if(typeof ns !== 'object' || typeof ns.default !== 'function') throw Error('namespace mismatch');"]).current_dir(&p.dir).output().unwrap();
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    p.file("dynamic.test.ts", "test('dynamic namespaces stay namespaces', async()=>{ const ns=await import('callable-package'); expect(typeof ns).toBe('object'); expect(ns.default).toBe(require('callable-package')); });");
    let dynamic = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "dynamic.test.ts"],
        &[],
    );
    assert_eq!(
        json_report(&dynamic)["numPassedTests"],
        1,
        "{}",
        dynamic.all()
    );
    std::fs::remove_file(p.dir.join("jest.config.js")).unwrap();
    p.file(
        "vitest.config.js",
        "export default { test: { environment: 'node' } };",
    );
    p.file("expected.json", r#"{"interop":true}"#);
    let esm = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "check.test.ts"],
        &[],
    );
    assert_eq!(json_report(&esm)["numPassedTests"], 2, "{}", esm.all());
}

#[test]
fn esbuild_only_fallback_keeps_static_dynamic_and_factory_scope_contracts() {
    let p = fixture(false, false);
    std::fs::remove_file(p.dir.join("node_modules/typescript")).unwrap();
    p.file("dynamic.test.ts", "test('dynamic namespace',async()=>{const ns=await import('callable-package');expect(typeof ns).toBe('object');expect(ns.default).toBe(require('callable-package'));});");
    p.file("dep.ts", "export const value=()=>0;");
    p.file("scope.test.ts", "import {value} from './dep'; const local=()=>42; jest.mock('./dep',()=>({value:()=>local()})); test('factory scope survives fallback',()=>expect(value()).toBe(42));");
    let run = turbo_test(
        &p.dir,
        &[
            "--jobs",
            "1",
            "--reporter",
            "json",
            "check.test.ts",
            "dynamic.test.ts",
            "scope.test.ts",
        ],
        &[("TURBO_NATIVE_CJS", "0")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 4, "{}", run.all());
    assert_eq!(json_report(&run)["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn metadata_fallback_preserves_dynamic_namespaces_and_commonjs_cycles() {
    let p = fixture(false, false);
    p.file("tsconfig.json",r#"{"compilerOptions":{"module":"commonjs","esModuleInterop":false,"experimentalDecorators":true,"emitDecoratorMetadata":true}}"#);
    p.file("metadata.test.ts","function marker(target:any){return target;} @marker class Model {} test('metadata dynamic namespace',async()=>{const ns=await import('callable-package');expect(typeof ns).toBe('object');expect(ns.default).toBe(require('callable-package'));expect(new Model()).toBeInstanceOf(Model);});");
    let run = turbo_test(
        &p.dir,
        &[
            "--jobs",
            "1",
            "--reporter",
            "json",
            "check.test.ts",
            "metadata.test.ts",
        ],
        &[("TURBO_NATIVE_CJS", "0")],
    );
    assert_eq!(json_report(&run)["numPassedTests"], 3, "{}", run.all());
}
#[test]
fn package_tsconfig_entries_match_the_project_typescript_oracle() {
    let p = fixture(false, false);
    p.file(
        "node_modules/compiler-preset/package.json",
        r#"{"name":"compiler-preset","main":"index.js","tsconfig":"configs/base.json"}"#,
    );
    p.file(
        "node_modules/compiler-preset/index.js",
        "module.exports={};",
    );
    p.file(
        "node_modules/compiler-preset/configs/base.json",
        r#"{"compilerOptions":{"module":"commonjs","esModuleInterop":false}}"#,
    );
    for extends in ["compiler-preset", "compiler-preset/configs/base.json"] {
        p.file("tsconfig.json", &format!(r#"{{"extends":"{extends}"}}"#));
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
    }
}
