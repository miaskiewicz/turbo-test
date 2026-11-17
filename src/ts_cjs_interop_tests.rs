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

#[test]
fn config_view_preserves_absent_malformed_and_non_utf8_config_errors() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let path = dir.join("tsconfig.json");
    let missing = ConfigFileSystem::for_config(Some(&path));
    assert_eq!(
        missing.read_to_string(&path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    for bytes in [
        b"{invalid json}".as_slice(),
        b"{/* unterminated".as_slice(),
        &[0xff, 0xfe, 0x80],
    ] {
        std::fs::write(&path, bytes).unwrap();
        let view = ConfigFileSystem::for_config(Some(&path));
        assert_eq!(view.read(&path).unwrap(), bytes);
        assert!(!enabled(&path));
        match std::str::from_utf8(bytes) {
            Ok(text) => assert_eq!(view.read_to_string(&path).unwrap(), text),
            Err(_) => assert_eq!(
                view.read_to_string(&path).unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            ),
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn config_view_preserves_relative_absolute_unknown_and_non_string_parents() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let path = dir.join("tsconfig.json");
    let base = dir.join("base.json");
    std::fs::write(&base, "{}").unwrap();
    let absolute = serde_json::to_string(&base).unwrap();
    let inputs = [
        "{}".to_owned(),
        r#"{"extends":false}"#.to_owned(),
        r#"{"extends":"missing-preset"}"#.to_owned(),
        r#"{"extends":"./base.json"}"#.to_owned(),
        format!("{{\"extends\":{absolute}}}"),
    ];
    for original in inputs {
        std::fs::write(&path, &original).unwrap();
        let view = ConfigFileSystem::for_config(Some(&path));
        assert_eq!(view.read_to_string(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn config_view_rewrites_only_resolved_package_entries_of_mixed_extends_arrays() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let path = dir.join("tsconfig.json");
    let package = dir.join("node_modules/preset");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"preset","tsconfig":"base.json"}"#,
    )
    .unwrap();
    std::fs::write(package.join("base.json"), "{}").unwrap();
    std::fs::write(dir.join("local.json"), "{}").unwrap();
    let original = r#"{ "extends": ["preset", "./local.json", "missing-preset", null, 42] }"#;
    std::fs::write(&path, original).unwrap();
    let view = ConfigFileSystem::for_config(Some(&path));
    let rewritten: Value = serde_json::from_str(&view.read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        rewritten["extends"][0],
        crate::canon::canonicalize(package.join("base.json"))
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(rewritten["extends"][1], "./local.json");
    assert_eq!(rewritten["extends"][2], "missing-preset");
    assert!(rewritten["extends"][3].is_null());
    assert_eq!(rewritten["extends"][4], 42);
    assert_eq!(view.read(&path).unwrap(), original.as_bytes());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn config_view_bounds_cycles_and_deep_parent_chains_without_rewriting_relative_configs() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let path = dir.join("tsconfig.json");
    let original = r#"{"extends":"./tsconfig.json"}"#;
    std::fs::write(&path, original).unwrap();
    assert_eq!(
        ConfigFileSystem::for_config(Some(&path))
            .read_to_string(&path)
            .unwrap(),
        original
    );
    for index in 0..35 {
        std::fs::write(
            dir.join(format!("chain-{index}.json")),
            format!("{{\"extends\":\"./chain-{}.json\"}}", index + 1),
        )
        .unwrap();
    }
    let first = dir.join("chain-0.json");
    let view = ConfigFileSystem::for_config(Some(&first));
    assert_eq!(
        view.read_to_string(&first).unwrap(),
        std::fs::read_to_string(&first).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn config_filesystem_new_delegates_raw_files_and_errors_without_a_config_overlay() {
    use oxc_resolver::FileSystem;
    let dir = directory();
    let path = dir.join("ordinary.json");
    let original = "// unchanged JSONC\n{\"extends\":\"unresolved-preset\",}\n";
    std::fs::write(&path, original).unwrap();
    let view = <ConfigFileSystem as FileSystem>::new();
    assert_eq!(view.read_to_string(&path).unwrap(), original);
    assert_eq!(view.read(&path).unwrap(), original.as_bytes());
    assert!(view.metadata(&path).unwrap().is_file());
    assert_eq!(
        view.read_to_string(&dir.join("absent.json"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn static_import_policy_preserves_non_require_calls_and_destructured_bindings() {
    let source="var __toESM = (mod, isNodeMode, target) => __copyProps(target, mod);\nconst {answer}=require('other');\nconst authored=__toESM(value);\nconst untouched=other(require('pkg'));".to_owned();
    assert_eq!(static_requires(source.clone()), source);
}

#[test]
fn destructured_authored_import_helper_is_never_rewritten_as_generated_code() {
    let source =
        "const { __toESM } = userHelpers; var ns = __toESM(require('package'));".to_string();
    assert_eq!(static_requires(source.clone()), source);
}
