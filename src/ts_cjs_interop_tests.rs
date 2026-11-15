use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "turbo-ts-interop-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
#[test]
fn jsonc_inheritance_uses_last_parent_then_child_and_tracks_edits() {
    let dir = directory();
    let config = dir.join("tsconfig.json");
    std::fs::write(dir.join("base.json"), r#"{"compilerOptions":{"module":"CommonJS","esModuleInterop":false},"url":"https://example.invalid"}"#).unwrap();
    std::fs::write(
        &config,
        "{ /* options from base */ \"extends\":\"./base.json\", }",
    )
    .unwrap();
    assert!(enabled(&config));
    std::fs::write(
        dir.join("override.json"),
        r#"{"compilerOptions":{"esModuleInterop":true}}"#,
    )
    .unwrap();
    std::fs::write(&config, r#"{"extends":["./base.json","./override.json"]}"#).unwrap();
    assert!(!enabled(&config));
    std::fs::write(&config,r#"{"extends":["./base.json","./override.json"],"compilerOptions":{"esModuleInterop":false}}"#).unwrap();
    assert!(enabled(&config));
    std::fs::write(
        &config,
        r#"{"extends":"./base.json","compilerOptions":{"module":"ESNext"}}"#,
    )
    .unwrap();
    assert!(!enabled(&config));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn invalid_missing_cyclic_and_unconfigured_inputs_do_not_enable_a_policy() {
    let dir = directory();
    let config = dir.join("tsconfig.json");
    assert!(!enabled(&config));
    for text in [
        "not json",
        "{}",
        r#"{"compilerOptions":{"module":"commonjs"}}"#,
        r#"{"extends":"./missing.json"}"#,
        r#"{"extends":"./tsconfig.json","compilerOptions":{"module":"commonjs","esModuleInterop":false}}"#,
    ] {
        std::fs::write(&config, text).unwrap();
        assert!(!enabled(&config));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_null_clears_inherited_compiler_options() {
    let dir = directory();
    let config = dir.join("tsconfig.json");
    std::fs::write(
        dir.join("base.json"),
        r#"{"compilerOptions":{"module":"commonjs","esModuleInterop":false}}"#,
    )
    .unwrap();
    for field in ["module", "esModuleInterop"] {
        std::fs::write(
            &config,
            format!(r#"{{"extends":"./base.json","compilerOptions":{{"{field}":null}}}}"#),
        )
        .unwrap();
        assert!(!enabled(&config));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn esbuild_static_wrapper_removal_preserves_dynamic_code_and_offsets() {
    let code = "var __toESM = (mod, isNodeMode, target) => __copyProps(target, mod);\nvar ns = __toESM(require('pkg'));\nvar dynamic = Promise.resolve().then(() => __toESM(require('pkg')));\nfunction nested() { var ns = __toESM(require('pkg')); }\n".to_owned();
    let output = static_requires(code.clone());
    assert_eq!(output.len(), code.len());
    assert_eq!(output.lines().count(), code.lines().count());
    assert!(!output.lines().nth(1).unwrap().contains("__toESM"));
    assert!(output
        .lines()
        .nth(3)
        .unwrap()
        .contains("__toESM(require('pkg'))"));
    assert!(output
        .lines()
        .nth(2)
        .unwrap()
        .contains("__toESM(require('pkg'))"));
    assert_eq!(
        static_requires("var ns = __toESM(other());".into()),
        "var ns = __toESM(other());"
    );
}

#[test]
#[cfg(unix)]
fn config_view_resolves_package_tsconfig_without_mutating_module_main_or_alias_roots() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let config = dir.join("tsconfig.json");
    let package = dir.join("node_modules/compiler-preset");
    std::fs::create_dir_all(package.join("configs")).unwrap();
    let original = r#"{"extends":"compiler-preset"}"#;
    std::fs::write(&config, original).unwrap();
    let metadata = r#"{"name":"compiler-preset","main":"index.js","tsconfig":"configs/base.json"}"#;
    std::fs::write(package.join("package.json"), metadata).unwrap();
    std::fs::write(package.join("index.js"), "module.exports={normal:true};").unwrap();
    std::fs::write(package.join("configs/base.json"),r#"{"compilerOptions":{"module":"commonjs","esModuleInterop":false,"baseUrl":".","paths":{"preset-alias":["./target.ts"]}}}"#).unwrap();
    std::fs::write(package.join("configs/target.ts"), "export const answer=42;").unwrap();
    let files = ConfigFileSystem::for_config(Some(&config));
    assert_eq!(files.read(&config).unwrap(), original.as_bytes());
    assert_ne!(files.read_to_string(&config).unwrap(), original);
    assert_eq!(
        files.read_to_string(&package.join("package.json")).unwrap(),
        metadata
    );
    assert!(files.metadata(&config).unwrap().is_file());
    assert!(files.symlink_metadata(&config).unwrap().is_file());
    assert_eq!(
        files.canonicalize(&config).unwrap(),
        crate::canon::canonicalize(&config).unwrap()
    );
    let link = dir.join("config-link");
    std::os::unix::fs::symlink(&config, &link).unwrap();
    assert_eq!(files.read_link(&link).unwrap(), config);
    let resolver = oxc_resolver::ResolverGeneric::new_with_file_system(
        files,
        oxc_resolver::ResolveOptions {
            extensions: vec![".ts".into(), ".js".into(), ".json".into()],
            tsconfig: Some(oxc_resolver::TsconfigDiscovery::Manual(
                oxc_resolver::TsconfigOptions {
                    config_file: config.clone(),
                    references: oxc_resolver::TsconfigReferences::Auto,
                },
            )),
            ..Default::default()
        },
    );
    assert_eq!(
        resolver.resolve(&dir, "compiler-preset").unwrap().path(),
        crate::canon::canonicalize(package.join("index.js")).unwrap()
    );
    assert_eq!(
        resolver.resolve(&dir, "preset-alias").unwrap().path(),
        crate::canon::canonicalize(package.join("configs/target.ts")).unwrap()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn authored_import_helper_names_are_not_generated_esbuild_provenance() {
    let source="function __toESM(value){return {user:true,value};} const wrapped=__toESM(require('node:path'));".to_owned();
    assert_eq!(static_requires(source.clone()), source);
}
