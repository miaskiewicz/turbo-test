//! Unit tests for runner.rs helpers in the resolution/config/transform/mock-text region.

use super::*;
use std::sync::atomic::AtomicUsize;

static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty, canonicalized temp directory for one test.
fn tmp(name: &str) -> PathBuf {
    let n = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join("turbo-test-unit-a");
    std::fs::create_dir_all(&base).unwrap();
    let dir = base
        .canonicalize()
        .unwrap()
        .join(format!("{name}-{}-{n}", std::process::id()));
    // leftover from an earlier process with the same pid: start empty
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `contents` at `dir/rel` (parents created); returns the path.
fn put(dir: &Path, rel: &str, contents: &str) -> PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
    p
}

// ---- TURBO_ALIAS / resolver options ------------------------------------------------------

#[test]
fn parse_alias_list_keeps_complete_records_only() {
    let raw = "@\u{1f}/abs/src\u{1e}~utils\u{1f}/abs/utils\u{1e}\u{1f}/no-key\u{1e}no-path\u{1f}\u{1e}no-separator";
    let path = |p: &str| vec![oxc_resolver::AliasValue::Path(p.to_string())];
    assert_eq!(
        parse_alias_list(raw),
        vec![
            ("@".to_string(), path("/abs/src")),
            ("~utils".to_string(), path("/abs/utils")),
        ]
    );
    assert!(parse_alias_list("").is_empty());
}

#[test]
fn config_alias_list_is_empty_without_env() {
    // The unit-test process never sets TURBO_ALIAS; the value is cached per thread.
    assert!(config_alias_list().is_empty());
    assert!(config_alias_list().is_empty());
}

#[test]
fn base_resolve_options_esm_first_and_cjs_first() {
    CJS_FIRST_RESOLUTION.with(|c| c.set(false));
    let o = base_resolve_options(None, true);
    assert_eq!(
        o.condition_names,
        vec!["import", "module", "browser", "default", "require"]
    );
    assert_eq!(o.main_fields, vec!["browser", "main", "module"]);
    assert!(o.tsconfig.is_none());
    assert!(o.extensions.contains(&".tsx".to_string()));
    // a require(): the `require` condition, `main` before `module` (turbo-test#134)
    let o = base_resolve_options(None, false);
    assert_eq!(o.condition_names, vec!["require", "browser", "default"]);
    assert_eq!(o.main_fields, vec!["browser", "main", "module"]);

    CJS_FIRST_RESOLUTION.with(|c| c.set(true));
    let o = base_resolve_options(Some(PathBuf::from("/p/tsconfig.json")), false);
    CJS_FIRST_RESOLUTION.with(|c| c.set(false));
    assert_eq!(o.condition_names, vec!["require", "node", "default"]);
    assert_eq!(o.main_fields, vec!["main", "module", "browser"]);
    match o.tsconfig {
        Some(oxc_resolver::TsconfigDiscovery::Manual(t)) => {
            assert_eq!(t.config_file, PathBuf::from("/p/tsconfig.json"))
        }
        other => panic!("expected a manual tsconfig, got {other:?}"),
    }
}

// ---- tsconfig / package type / ESM detection ---------------------------------------------

#[test]
fn nearest_tsconfig_walks_up_and_stops_at_a_node_modules_package_dir() {
    let d = tmp("tsc");
    put(&d, "tsconfig.json", "{}");
    put(&d, "src/deep/a.ts", "");
    assert_eq!(
        nearest_tsconfig_uncached(&d.join("src/deep")),
        Some(d.join("tsconfig.json"))
    );
    // memoized wrapper agrees (twice: miss then hit)
    assert_eq!(
        nearest_tsconfig(&d.join("src/deep")),
        Some(d.join("tsconfig.json"))
    );
    assert_eq!(
        nearest_tsconfig(&d.join("src/deep")),
        Some(d.join("tsconfig.json"))
    );
    // a nearer tsconfig wins
    put(&d, "src/tsconfig.json", "{}");
    assert_eq!(
        nearest_tsconfig_uncached(&d.join("src/deep")),
        Some(d.join("src/tsconfig.json"))
    );
    // a `node_modules` dir that carries a package.json is a hard stop
    put(&d, "node_modules/package.json", "{}");
    put(&d, "node_modules/pkg/package.json", "{}");
    std::fs::create_dir_all(d.join("node_modules/pkg/lib")).unwrap();
    assert_eq!(
        nearest_tsconfig_uncached(&d.join("node_modules/pkg/lib")),
        None
    );
}

#[test]
fn is_esm_module_by_extension_and_package_type() {
    let d = tmp("esm");
    for (f, esm) in [
        ("a.mjs", true),
        ("a.mts", true),
        ("a.ts", true),
        ("a.tsx", true),
        ("a.jsx", true),
        ("a.cjs", false),
        ("a.cts", false),
    ] {
        assert_eq!(is_esm_module_uncached(&d.join(f)), esm, "{f}");
    }
    put(&d, "m/package.json", r#"{ "type": "module" }"#);
    put(&d, "m2/package.json", r#"{"type":"module"}"#);
    put(&d, "c/package.json", r#"{ "name": "c" }"#);
    assert!(is_esm_module_uncached(&d.join("m/x.js")));
    assert!(is_esm_module_uncached(&d.join("m2/sub/x.js")));
    assert!(!is_esm_module_uncached(&d.join("c/x.js")));
    // no package.json anywhere up the tree -> CommonJS
    assert!(!is_esm_module_uncached(Path::new(
        "/turbo-test-no-such-dir/sub/x.js"
    )));
    // memoized wrapper: miss then hit
    assert!(is_esm_module(&d.join("m/y.js")));
    assert!(is_esm_module(&d.join("m/y.js")));
    assert!(!is_esm_module(&d.join("c/y.js")));
}

#[test]
fn nearest_pkg_type_reads_the_closest_package_json() {
    let d = tmp("pkgtype");
    put(&d, "m/package.json", r#"{ "type": "module" }"#);
    put(&d, "m2/package.json", r#"{"type":"module"}"#);
    put(&d, "c/package.json", r#"{ "type": "commonjs" }"#);
    put(&d, "c2/package.json", r#"{"type":"commonjs"}"#);
    put(&d, "n/package.json", r#"{ "name": "n" }"#);
    put(&d, "m/inner/package.json", r#"{ "name": "inner" }"#);
    assert_eq!(nearest_pkg_type_uncached(&d.join("m/a.js")), Some("module"));
    assert_eq!(
        nearest_pkg_type_uncached(&d.join("m2/a.js")),
        Some("module")
    );
    assert_eq!(
        nearest_pkg_type_uncached(&d.join("c/a.js")),
        Some("commonjs")
    );
    assert_eq!(
        nearest_pkg_type_uncached(&d.join("c2/a.js")),
        Some("commonjs")
    );
    assert_eq!(nearest_pkg_type_uncached(&d.join("n/a.js")), None);
    // the nearest package.json decides, even without a type
    assert_eq!(nearest_pkg_type_uncached(&d.join("m/inner/a.js")), None);
    assert_eq!(nearest_pkg_type(&d.join("c/b.js")), Some("commonjs"));
    // no package.json up to the filesystem root; and a path without a parent directory
    assert_eq!(
        nearest_pkg_type_uncached(Path::new("/turbo-test-no-such-dir/sub/a.js")),
        None
    );
    assert_eq!(nearest_pkg_type(Path::new("/")), None);
    assert_eq!(nearest_pkg_type(&d.join("c/b.js")), Some("commonjs"));
    // no package.json up to the filesystem root; and a path without a parent directory
    assert_eq!(
        nearest_pkg_type_uncached(Path::new("/turbo-test-no-such-dir/sub/a.js")),
        None
    );
    assert_eq!(nearest_pkg_type(Path::new("/")), None);
}

#[test]
fn detect_kind_by_extension_and_package_type() {
    let d = tmp("kind");
    assert_eq!(detect_kind(&d.join("a.cjs")), Kind::Cjs);
    for f in ["a.mjs", "a.mts", "a.ts", "a.tsx", "a.jsx"] {
        assert_eq!(detect_kind(&d.join(f)), Kind::Esm, "{f}");
    }
    put(&d, "m/package.json", r#"{"type":"module"}"#);
    let m = put(&d, "m/a.js", "module.exports = 1;");
    assert_eq!(detect_kind(&m), Kind::Esm);
    put(&d, "c/package.json", r#"{"type":"commonjs"}"#);
    let c = put(&d, "c/a.js", "export default 1;");
    assert_eq!(detect_kind(&c), Kind::Cjs);
    // no explicit type -> Node's syntax detection: ESM syntax is ESM, the rest CommonJS
    put(&d, "s/package.json", r#"{"name":"s"}"#);
    let cases = [
        ("cjs1.js", "module.exports = 1;", Kind::Cjs),
        ("cjs2.js", "exports.a = 1;", Kind::Cjs),
        ("cjs3.js", "exports['a'] = 1;", Kind::Cjs),
        ("cjs4.js", "const x = require('x');", Kind::Cjs),
        ("esm1.js", "import x from 'x';", Kind::Esm),
        ("esm2.js", "import{x}from'x';", Kind::Esm),
        ("esm3.js", "export const a = 1;", Kind::Esm),
        ("esm4.js", "export{a};", Kind::Esm),
        ("esm5.js", "export*from'x';", Kind::Esm),
        ("esm6.js", "x;export default 1", Kind::Esm),
        (
            "mixed.js",
            "import x from 'x'; module.exports = x;",
            Kind::Esm,
        ),
        ("plain.js", "var a = 1;", Kind::Cjs),
        (
            "comment.js",
            "// import these where needed\nexports.a = 1;",
            Kind::Cjs,
        ),
        ("tla.js", "await Promise.resolve();", Kind::Esm),
    ];
    for (f, src, k) in cases {
        let p = put(&d, &format!("s/{f}"), src);
        assert_eq!(detect_kind(&p), k, "{f}");
    }
    // unreadable file -> empty source -> CommonJS
    assert_eq!(detect_kind(&d.join("s/missing.js")), Kind::Cjs);
}

// ---- project kind / resolution -------------------------------------------------------------

#[test]
fn cjs_first_project_detection() {
    let d = tmp("cjsfirst");
    // vitest config -> ESM-first, even next to a jest config
    put(&d, "v/vitest.config.ts", "export default {}");
    put(&d, "v/jest.config.js", "module.exports = {}");
    assert!(!cjs_first_project_uncached(&d.join("v/a.test.ts")));
    put(&d, "vite/vite.config.js", "export default {}");
    assert!(!cjs_first_project_uncached(&d.join("vite/a.test.ts")));
    // jest config with the default (node) environment -> CJS-first, also from a nested dir
    put(
        &d,
        "j/jest.config.js",
        "module.exports = { testEnvironment: 'node' }",
    );
    assert!(cjs_first_project_uncached(
        &d.join("j/sub/deeper/a.test.ts")
    ));
    put(&d, "j2/jest.config.json", "{}");
    assert!(cjs_first_project_uncached(&d.join("j2/a.test.ts")));
    // jest config with a DOM env -> ESM-first
    for env in ["jsdom", "happy-dom", "jest-environment-jsdom"] {
        let sub = format!("dom-{env}");
        put(
            &d,
            &format!("{sub}/jest.config.ts"),
            &format!("export default {{ testEnvironment: '{env}' }}"),
        );
        assert!(
            !cjs_first_project_uncached(&d.join(format!("{sub}/a.test.ts"))),
            "{env}"
        );
    }
    // a package.json "jest" block is a jest config too (turbo-test#31 item 1); its own
    // testEnvironment decides, not a same-named key elsewhere in package.json
    put(
        &d,
        "pj/package.json",
        r#"{"name":"p","jest":{"testEnvironment":"node"}}"#,
    );
    assert!(cjs_first_project_uncached(&d.join("pj/a.test.ts")));
    put(
        &d,
        "pjdom/package.json",
        r#"{"name":"p","jest":{"testEnvironment":"jsdom"}}"#,
    );
    assert!(!cjs_first_project_uncached(&d.join("pjdom/a.test.ts")));
    put(
        &d,
        "pjo/package.json",
        r#"{"name":"p","x":{"testEnvironment":"jsdom"},"jest":{}}"#,
    );
    assert!(cjs_first_project_uncached(&d.join("pjo/a.test.ts")));
    // a package.json without a jest block is not a jest config
    put(
        &d,
        "plain/package.json",
        r#"{"name":"p","scripts":{"jest":"jest"}}"#,
    );
    assert!(!cjs_first_project_uncached(&d.join("plain/a.test.ts")));
    // memoized wrapper: miss then hit, same answer
    assert!(cjs_first_project(&d.join("j/b.test.ts")));
    assert!(cjs_first_project(&d.join("j/c.test.ts")));
    assert!(!cjs_first_project(&d.join("v/b.test.ts")));
}

#[test]
fn resolve_spec_relative_bare_query_and_fallback_probing() {
    let d = tmp("resolve");
    let a = put(&d, "src/a.ts", "export const a = 1;");
    let idx = put(&d, "src/dir/index.js", "module.exports = 1;");
    let json = put(&d, "src/data.json", "{}");
    let png = put(&d, "src/img.png", "");
    let svg = put(&d, "src/icon.svg", "<svg/>");
    let mts = put(&d, "src/only.mts", "export {};");
    let src = d.join("src");
    assert_eq!(resolve_spec("./a", &src), Some(a.clone()));
    assert_eq!(resolve_spec("./dir", &src), Some(idx.clone()));
    // `?query` suffixes resolve the base file
    assert_eq!(resolve_spec("./data.json?raw", &src), Some(json.clone()));
    assert_eq!(resolve_spec("./img.png?url", &src), Some(png.clone()));
    // `react` must be a bare flag token to select the svgr stub; as a value it is just a query
    assert_eq!(
        resolve_spec("./icon.svg?foo=react", &src),
        Some(svg.clone())
    );
    assert_eq!(resolve_spec("./img.png?react", &src), Some(png.clone()));
    // oxc's extension list has no .mts -> manual probing finds it
    assert_eq!(
        resolve_spec_as_uncached("./only", &src, true),
        Some(mts.clone())
    );
    // absolute spec
    assert_eq!(
        resolve_spec_as_uncached(&a.to_string_lossy(), &d, false),
        Some(a.clone())
    );
    assert_eq!(resolve_spec("./missing", &src), None);
    assert_eq!(resolve_spec("not-a-package", &src), None);
    // memo: a second lookup is answered identically
    assert_eq!(resolve_spec("./a", &src), Some(a));
}

#[test]
fn resolve_spec_bare_package_conditions_follow_cjs_first() {
    let d = tmp("resolve-pkg");
    put(
        &d,
        "node_modules/dual/package.json",
        r#"{ "name": "dual", "exports": { ".": { "import": "./esm.mjs", "require": "./cjs.js" } } }"#,
    );
    let esm = put(&d, "node_modules/dual/esm.mjs", "export default 1;");
    let cjs = put(&d, "node_modules/dual/cjs.js", "module.exports = 1;");
    let deep = put(&d, "node_modules/plain/lib/x.js", "module.exports = 1;");
    put(
        &d,
        "node_modules/plain/package.json",
        r#"{ "name": "plain" }"#,
    );
    CJS_FIRST_RESOLUTION.with(|c| c.set(false));
    assert_eq!(
        resolve_spec_as_uncached("dual", &d, true),
        Some(esm.clone())
    );
    // a require() takes the `require` build (turbo-test#134) ...
    assert_eq!(
        resolve_spec_as_uncached("dual", &d, false),
        Some(cjs.clone())
    );
    // ... and an import-only package as an import
    put(
        &d,
        "node_modules/esm-only/package.json",
        r#"{ "name": "esm-only", "exports": { "import": "./i.mjs" } }"#,
    );
    let only = put(&d, "node_modules/esm-only/i.mjs", "export default 1;");
    assert_eq!(resolve_spec_as_uncached("esm-only", &d, false), Some(only));
    // `main` before `module`
    put(
        &d,
        "node_modules/modfield/package.json",
        r#"{ "name": "modfield", "main": "./c.js", "module": "./e.mjs" }"#,
    );
    let main = put(&d, "node_modules/modfield/c.js", "module.exports = 1;");
    put(&d, "node_modules/modfield/e.mjs", "export default 1;");
    assert_eq!(resolve_spec_as_uncached("modfield", &d, true), Some(main));
    CJS_FIRST_RESOLUTION.with(|c| c.set(true));
    let got = resolve_spec_as_uncached("dual", &d, false);
    CJS_FIRST_RESOLUTION.with(|c| c.set(false));
    assert_eq!(got, Some(cjs));
    // `node_modules/<pkg>/...` is a bare specifier in disguise
    assert_eq!(
        resolve_spec_as_uncached("node_modules/plain/lib/x", &d.join("sub"), true),
        Some(deep)
    );
}

#[test]
fn resolver_for_is_cached_per_tsconfig_kind_and_mode() {
    let d = tmp("resolver-for");
    put(&d, "tsconfig.json", "{}");
    let r1 = resolver_for(&d, true);
    let r2 = resolver_for(&d, true);
    assert!(std::rc::Rc::ptr_eq(&r1, &r2));
    let r3 = resolver_for(&d, false);
    assert!(!std::rc::Rc::ptr_eq(&r1, &r3));
}

#[test]
fn is_asset_extensions() {
    for f in [
        "a.css", "a.scss", "a.sass", "a.less", "a.styl", "a.svg", "a.png", "a.jpg", "a.jpeg",
        "a.gif", "a.webp", "a.avif", "a.ico", "a.woff", "a.woff2", "a.ttf", "a.eot", "a.otf",
        "a.mp4", "a.webm", "a.wav", "a.mp3", "a.txt", "a.md",
    ] {
        assert!(is_asset(Path::new(f)), "{f}");
    }
    for f in ["a.js", "a.ts", "a.json", "a.node", "noext"] {
        assert!(!is_asset(Path::new(f)), "{f}");
    }
}

// ---- __dirname injection + identifier helpers ---------------------------------------------

#[test]
fn inject_dirname_esm_only_when_referenced_and_undeclared() {
    let abs = Path::new("/proj/src/file.ts");
    assert_eq!(inject_dirname_esm(abs, "export const a = 1;"), None);
    assert_eq!(
        inject_dirname_esm(abs, "export const d = __dirname;").as_deref(),
        Some("const __dirname=\"/proj/src\";export const d = __dirname;")
    );
    assert_eq!(
        inject_dirname_esm(abs, "export const f = __filename;").as_deref(),
        Some("const __filename=\"/proj/src/file.ts\";export const f = __filename;")
    );
    assert_eq!(
        inject_dirname_esm(abs, "f(__filename, __dirname)").as_deref(),
        Some("const __filename=\"/proj/src/file.ts\";const __dirname=\"/proj/src\";f(__filename, __dirname)")
    );
    // declared locally (bundle shim) -> untouched
    assert_eq!(
        inject_dirname_esm(abs, "var __dirname = '/x'; use(__dirname);"),
        None
    );
    // only a member access -> no global reference
    assert_eq!(inject_dirname_esm(abs, "x.__dirname + y.__filename"), None);
    // a root-level file uses `/` as its directory
    assert_eq!(
        inject_dirname_esm(Path::new("/"), "__dirname").as_deref(),
        Some("const __dirname=\"/\";__dirname")
    );
    // path metacharacters are escaped
    let odd = Path::new("/p/a\"b\\c/f.ts");
    assert_eq!(
        inject_dirname_esm(odd, "__dirname").as_deref(),
        Some("const __dirname=\"/p/a\\\"b\\\\c\";__dirname")
    );
}

#[test]
fn references_and_declares_ident() {
    assert!(references_ident("__dirname", "__dirname"));
    assert!(references_ident("(__dirname)", "__dirname"));
    assert!(!references_ident("a__dirname", "__dirname"));
    assert!(!references_ident("__dirnameX", "__dirname"));
    assert!(!references_ident("x.__dirname", "__dirname"));
    assert!(!references_ident("", "__dirname"));
    // a later standalone occurrence after rejected ones
    assert!(references_ident(
        "x.__dirname; $__dirname; __dirname",
        "__dirname"
    ));

    assert!(declares_ident("const __dirname = 1", "__dirname"));
    assert!(declares_ident("let __dirname;", "__dirname"));
    assert!(declares_ident("var __dirname", "__dirname"));
    assert!(!declares_ident("let __dirnameX = 1", "__dirname"));
    assert!(declares_ident(
        "let __dirnameX = 1; const __dirname=2",
        "__dirname"
    ));
    assert!(!declares_ident("__dirname = 1", "__dirname"));
    assert!(is_ident_byte(b'$') && is_ident_byte(b'_') && is_ident_byte(b'z'));
    assert!(!is_ident_byte(b'.') && !is_ident_byte(b' '));
}

#[test]
fn js_string_literal_escapes() {
    assert_eq!(js_string_literal("plain"), "\"plain\"");
    assert_eq!(
        js_string_literal("a\\b\"c\nd\re"),
        "\"a\\\\b\\\"c\\nd\\re\""
    );
    assert_eq!(js_string_literal(""), "\"\"");
}

// ---- cache helpers --------------------------------------------------------------------------

#[test]
fn write_atomic_publishes_whole_file_without_temp_leftovers() {
    let d = tmp("atomic");
    let p = d.join("out.js");
    write_atomic(&p, "hello");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
    write_atomic(&p, "replaced");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "replaced");
    let names: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["out.js".to_string()]);
    // unwritable destination (missing parent): silently skipped
    let missing = d.join("no/such/dir/out.js");
    write_atomic(&missing, "x");
    assert!(!missing.exists());
}

#[test]
fn cache_key_depends_on_source_and_extension() {
    let ts = Path::new("/a/x.ts");
    let k = cache_key(ts, "src");
    assert_eq!(k, cache_key(Path::new("/other/y.ts"), "src"));
    assert_ne!(k, cache_key(ts, "src2"));
    assert_ne!(k, cache_key(Path::new("/a/x.tsx"), "src"));
    assert_ne!(
        cache_key(Path::new("/a/noext"), "src"),
        cache_key(Path::new("/a/x.js"), "src")
    );
}

#[test]
fn transform_cache_stats_are_monotonic_counters() {
    let (h0, m0) = transform_cache_stats();
    let (h1, m1) = transform_cache_stats();
    assert!(h1 >= h0 && m1 >= m0);
}

#[test]
fn project_root_and_tsc_root_find_their_markers() {
    let d = tmp("roots");
    put(&d, "node_modules/.bin/esbuild", "");
    put(&d, "node_modules/typescript/package.json", "{}");
    let f = put(&d, "a/b/c.ts", "");
    assert_eq!(project_root_uncached(&f), Some(d.clone()));
    assert_eq!(project_root(&f), Some(d.clone()));
    assert_eq!(project_root(&f), Some(d.clone()));
    assert_eq!(tsc_root(&f), Some(d.clone()));
    let lone = tmp("roots-none");
    let g = put(&lone, "x/y.ts", "");
    assert_eq!(project_root_uncached(&g), None);
    assert_eq!(tsc_root(&g), None);
}

#[test]
fn spec_basename_takes_the_last_segment() {
    assert_eq!(spec_basename("../../x/analytics"), "analytics");
    assert_eq!(spec_basename("pkg/"), "pkg");
    assert_eq!(spec_basename("plain"), "plain");
    assert_eq!(spec_basename("@scope/pkg"), "pkg");
}

#[test]
fn esbuild_bundle_full_uses_the_root_override() {
    // The file's own project has an esbuild marker, but the override root has no esbuild
    // binary: the override wins, esbuild can't be spawned, and bundling reports None.
    let proj = tmp("bundle-own");
    put(&proj, "node_modules/.bin/esbuild", "");
    let f = put(&proj, "a.ts", "export const a = 1;");
    let other = tmp("bundle-override");
    assert_eq!(
        esbuild_bundle_full(&f, &[], &HashMap::new(), Some(&other)),
        None
    );
}

#[test]
fn publish_bundle_renames_or_reuses_an_existing_output() {
    let d = tmp("publish");
    // normal case: the temp file becomes the output
    let tmp1 = put(&d, "b.tmp1", "bundle");
    let out1 = d.join("b.mjs");
    assert!(publish_bundle(&tmp1, &out1));
    assert_eq!(std::fs::read_to_string(&out1).unwrap(), "bundle");
    assert!(!tmp1.exists());
    // rename fails (the output is a non-empty directory), but an output exists: reuse it and
    // clean up the temp
    let tmp2 = put(&d, "c.tmp2", "bundle");
    let out2 = d.join("c.mjs");
    put(&out2, "occupied", "");
    assert!(publish_bundle(&tmp2, &out2));
    assert!(!tmp2.exists());
    // rename fails and there is no output: unusable
    assert!(!publish_bundle(
        &d.join("missing.tmp"),
        &d.join("missing.mjs")
    ));
}

// ---- environment pragmas / reuse / config parsing ------------------------------------------

#[test]
fn env_pragma_variants_and_forced_env() {
    assert_eq!(
        file_env_pragma("/* @vitest-environment\thappy-dom */").as_deref(),
        Some("happy-dom")
    );
    assert_eq!(file_env_pragma("// @vitest-environment"), None);
    assert_eq!(
        forced_env("// @vitest-environment Node").as_deref(),
        Some("node")
    );
}

#[test]
fn needs_dom_honours_pragmas_extension_and_content() {
    let d = tmp("needsdom");
    let node_tsx = put(&d, "a.test.tsx", "// @vitest-environment node\n");
    assert!(!needs_dom(&node_tsx));
    for env in ["jsdom", "happy-dom", "happydom"] {
        let f = put(
            &d,
            &format!("{env}.test.ts"),
            &format!("// @vitest-environment {env}\n"),
        );
        assert!(needs_dom(&f), "{env}");
    }
    // an unknown environment falls through to the heuristic
    let edge = put(&d, "edge.test.ts", "// @vitest-environment edge-runtime\n");
    assert!(
        needs_dom(&edge),
        "the pragma text itself mentions @vitest-environment"
    );
    assert!(needs_dom(&put(&d, "x.test.jsx", "")));
    assert!(needs_dom(&put(&d, "x.test.tsx", "")));
    for (i, s) in [
        "import '@testing-library/react'",
        "react-dom",
        "ReactDOM.render",
        "document.body",
        "window.foo",
        "getComputedStyle(x)",
        "happy-dom",
        "jsdom",
    ]
    .iter()
    .enumerate()
    {
        assert!(needs_dom(&put(&d, &format!("c{i}.test.ts"), s)), "{s}");
    }
    assert!(!needs_dom(&put(&d, "plain.test.ts", "test('x', () => {})")));
    assert!(!needs_dom(&d.join("missing.test.ts")));
}

#[test]
fn vitest_isolate_false_detection() {
    let d = tmp("isolate");
    put(&d, "package.json", "{}");
    put(
        &d,
        "vitest.config.ts",
        "export default { test: { isolate : false } }",
    );
    assert!(vitest_isolate_false(&d.join("src/a.test.ts")));
    let c = tmp("isolate-comment");
    put(&c, "package.json", "{}");
    put(
        &c,
        "vitest.config.ts",
        "// isolate: false would be faster\nexport default { test: { isolate: true } }",
    );
    assert!(!vitest_isolate_false(&c.join("a.test.ts")));
    // the project root (package.json) stops the walk: a parent's config is not consulted
    let p = tmp("isolate-parent");
    put(
        &p,
        "vite.config.ts",
        "export default { test: { isolate: false } }",
    );
    put(&p, "child/package.json", "{}");
    assert!(!vitest_isolate_false(&p.join("child/a.test.ts")));
    // ...but without one the walk continues upward
    assert!(vitest_isolate_false(&p.join("nopkg/a.test.ts")));
}

#[test]
fn find_config_key_skips_comments_and_non_keys() {
    let s = "// setupFiles: ['x']\nconst setupFiles = 1;\n  setupFiles : ['./s.ts']";
    let at = find_config_key(s, "setupFiles").unwrap();
    assert!(s[at..].starts_with("setupFiles : ["));
    assert_eq!(find_config_key("setupFiles = 3", "setupFiles"), None);
    assert_eq!(find_config_key("nothing here", "setupFiles"), None);
    assert_eq!(find_config_key("  // jest: {}", "jest"), None);
}

#[test]
fn find_config_key_reads_quoted_json_keys() {
    // turbo-test#31 item 1: JSON keys are quoted, so the quote sits between key and ':'
    let s = "{\"jest\":{\"setupFiles\":[]}}";
    assert_eq!(find_config_key(s, "jest"), Some(2));
    assert_eq!(find_config_key(s, "setupFiles"), Some(10));
    assert_eq!(find_config_key("{ 'jest' : {} }", "jest"), Some(3));
    // mismatched or one-sided quotes, a longer key, or a key that is only a suffix: no match
    assert_eq!(find_config_key("{\"jest': {}}", "jest"), None);
    assert_eq!(find_config_key("{jest\": {}}", "jest"), None);
    assert_eq!(find_config_key("{\"jestx\": {}}", "jest"), None);
    assert_eq!(find_config_key("{ myjest: {} }", "jest"), None);
    assert_eq!(find_config_key("{\"myjest\": {}}", "jest"), None);
}

#[test]
fn package_json_jest_block_is_the_top_level_jest_object_only() {
    assert_eq!(
        package_json_jest_block(
            r#"{"scripts":{"jest":"jest"},"name":"x","jest":{"a":{"b":"}"},"c":1},"z":{"setupFiles":[]}}"#
        )
        .as_deref(),
        Some(r#"{"a":{"b":"}"},"c":1}"#)
    );
    assert_eq!(
        package_json_jest_block(r#"{ "jest" : { } }"#).as_deref(),
        Some("{}")
    );
    // no block, a nested-only `jest` key, a non-object value, or invalid JSON
    assert_eq!(package_json_jest_block(r#"{"name":"x"}"#), None);
    assert_eq!(package_json_jest_block(r#"{"scripts":{"jest":{}}}"#), None);
    assert_eq!(
        package_json_jest_block(r#"{"jest":"./jest.config.js"}"#),
        None
    );
    assert_eq!(package_json_jest_block(r#"{"jest":{"a":1"#), None);
}

#[test]
fn runtime_config_reads_fake_timers_and_the_mock_switches() {
    // turbo-test#180 / #183: what __ttApplyConfig gets from the vitest config's text
    assert_eq!(runtime_config_literal("export default { test: {} }"), None);
    assert_eq!(
        runtime_config_literal(
            "test: { fakeTimers: { now: 1, toFake: ['Date'] }, restoreMocks: true, clearMocks: false, unstubGlobals:true }"
        )
        .as_deref(),
        Some("{ fakeTimers: { now: 1, toFake: ['Date'] }, restoreMocks: true, unstubGlobals: true }")
    );
    assert_eq!(
        runtime_config_literal("mockReset: true, unstubEnvs: true, fakeTimers: defaults")
            .as_deref(),
        Some("{ mockReset: true, unstubEnvs: true }")
    );
    assert_eq!(
        config_object_literal("fakeTimers: { a: { b: 1 } }, x", "fakeTimers").as_deref(),
        Some("{ a: { b: 1 } }")
    );
    assert_eq!(
        config_object_literal("fakeTimers: { open", "fakeTimers"),
        None
    );
    assert!(!config_is_true("clearMocks: 'true'", "clearMocks"));
}

#[test]
fn config_string_value_quotes() {
    assert_eq!(
        config_string_value("rootDir: '<x>'", "rootDir").as_deref(),
        Some("<x>")
    );
    // JSON (quoted) keys, double or single quoted (turbo-test#31 item 1)
    assert_eq!(
        config_string_value("\"rootDir\" : \"src\"", "rootDir").as_deref(),
        Some("src")
    );
    assert_eq!(
        config_string_value("{ 'rootDir': 'lib' }", "rootDir").as_deref(),
        Some("lib")
    );
    assert_eq!(
        config_string_value("{ rootDir: \"src\" }", "rootDir").as_deref(),
        Some("src")
    );
    assert_eq!(
        config_string_value("rootDir: `tpl`", "rootDir").as_deref(),
        Some("tpl")
    );
    assert_eq!(
        config_string_value("rootDir: path.join(a)", "rootDir"),
        None
    );
    assert_eq!(config_string_value("rootDir: 'open", "rootDir"), None);
    assert_eq!(config_string_value("rootDir:", "rootDir"), None);
    assert_eq!(config_string_value("other: 'x'", "rootDir"), None);
}

#[test]
fn normalize_path_is_lexical() {
    assert_eq!(
        normalize_path(Path::new("/a/b/../c/./d")),
        PathBuf::from("/a/c/d")
    );
    assert_eq!(normalize_path(Path::new("a/./b/..")), PathBuf::from("a"));
    assert_eq!(normalize_path(Path::new("/..")), PathBuf::from("/"));
    // a leading `.` is the only CurDir component `components()` yields
    assert_eq!(normalize_path(Path::new("./a/b")), PathBuf::from("a/b"));
}

#[test]
fn vitest_setup_files_from_vitest_config() {
    let d = tmp("setup-vitest");
    let s1 = put(&d, "s1.ts", "");
    let s2 = put(&d, "test/s2.js", "");
    put(
        &d,
        "vitest.config.ts",
        "// setupFiles: ['./ignored.ts']\nexport default { test: { setupFiles: ['./s1.ts', \"./test/s2\", `./missing.ts`, ] } }",
    );
    assert_eq!(
        vitest_setup_files_uncached(&d.join("src/a.test.ts")),
        vec![s1.clone(), s2.clone()]
    );
    // memoized wrapper (miss then hit)
    assert_eq!(
        vitest_setup_files(&d.join("src/b.test.ts")),
        vec![s1.clone(), s2]
    );
    assert_eq!(vitest_setup_files(&d.join("src/b.test.ts")).len(), 2);

    // config present but no setupFiles / only unresolvable ones / not an array -> empty
    for (name, cfg) in [
        ("none", "export default { test: {} }"),
        (
            "unresolved",
            "export default { test: { setupFiles: ['./nope.ts'] } }",
        ),
        (
            "scalar",
            "export default { test: { setupFiles: './s1.ts' } }",
        ),
        ("open", "export default { test: { setupFiles: ['./s1.ts' "),
    ] {
        let e = tmp(&format!("setup-{name}"));
        put(&e, "s1.ts", "");
        put(&e, "vitest.config.mts", cfg);
        assert!(
            vitest_setup_files_uncached(&e.join("a.test.ts")).is_empty(),
            "{name}"
        );
    }
}

/// turbo-test#43 item 1: setupFiles are read from every config name vitest 4 resolves
/// (`vitest.config` then `vite.config`, each `.ts .mts .cts .js .mjs .cjs`), the first one in
/// that order winning — checked against vitest 4.1.12.
#[test]
fn vitest_setup_files_follow_vitest_config_resolution_order() {
    for name in [
        "vitest.config.ts",
        "vitest.config.mts",
        "vitest.config.cts",
        "vitest.config.js",
        "vitest.config.mjs",
        "vitest.config.cjs",
        "vite.config.ts",
        "vite.config.mts",
        "vite.config.cts",
        "vite.config.js",
        "vite.config.mjs",
        "vite.config.cjs",
    ] {
        let d = tmp(&format!("setup-name-{name}"));
        let s = put(&d, "setup.js", "");
        put(
            &d,
            name,
            "export default { test: { setupFiles: ['./setup.js'] } }",
        );
        assert_eq!(
            vitest_setup_files_uncached(&d.join("a.test.ts")),
            vec![s],
            "{name}"
        );
    }
    // precedence: vitest.config.js before vite.config.ts, vitest.config.cts before .js
    for (first, second) in [
        ("vitest.config.js", "vite.config.ts"),
        ("vitest.config.cts", "vitest.config.js"),
        ("vite.config.cjs", "vitest.config.cjs"),
    ] {
        let d = tmp(&format!("setup-prec-{first}-{second}"));
        let a = put(&d, "a.js", "");
        let b = put(&d, "b.js", "");
        put(
            &d,
            first,
            "export default { test: { setupFiles: ['./a.js'] } }",
        );
        put(
            &d,
            second,
            "export default { test: { setupFiles: ['./b.js'] } }",
        );
        let want = if first.starts_with("vitest") { a } else { b };
        assert_eq!(
            vitest_setup_files_uncached(&d.join("a.test.ts")),
            vec![want],
            "{first} {second}"
        );
    }
}

/// turbo-test#49: a project with any vitest config name is a vitest project (ESM-first), and
/// `isolate: false` is read from each of them.
#[test]
fn every_vitest_config_name_marks_a_vitest_project() {
    for name in [
        "vitest.config.cjs",
        "vitest.config.cts",
        "vite.config.mts",
        "vite.config.cjs",
    ] {
        let d = tmp(&format!("cfg-esm-{name}"));
        put(&d, "package.json", "{}");
        // (a jest config with the node environment would make it CJS-first)
        put(
            &d,
            "jest.config.js",
            "module.exports = { testEnvironment: 'node' }",
        );
        put(&d, name, "module.exports = { test: { isolate: false } }");
        assert!(!cjs_first_project_uncached(&d.join("a.test.ts")), "{name}");
        assert!(vitest_isolate_false(&d.join("a.test.ts")), "{name}");
    }
}

#[test]
fn vitest_setup_files_falls_back_to_jest_config_up_the_tree() {
    let d = tmp("setup-jest-walk");
    let s = put(&d, "jest.setup.js", "");
    put(
        &d,
        "jest.config.js",
        "module.exports = { setupFilesAfterEnv: ['<rootDir>/jest.setup.js'] }",
    );
    assert_eq!(
        vitest_setup_files_uncached(&d.join("pkg/src/a.test.ts")),
        vec![s]
    );
    let none = tmp("setup-nothing");
    assert!(vitest_setup_files_uncached(&none.join("a.test.ts")).is_empty());
}

#[test]
fn jest_setup_files_root_dir_absolute_and_resolution() {
    let d = tmp("jest-setup");
    let a = put(&d, "config/a.js", "");
    let b = put(&d, "b.ts", "");
    let c = put(&d, "c/index.js", "");
    let abs = put(&d, "abs/setup.js", "");
    put(
        &d,
        "jest.config.cjs",
        &format!(
            "module.exports = {{ rootDir: './config/..', setupFiles: ['<rootDir>/config/a.js', './b', ''], setupFilesAfterEnv: [\"{}\", './c', './gone.js'] }}",
            abs.display()
        ),
    );
    assert_eq!(jest_setup_files(&d), Some(vec![a, b, abs, c]));

    // JSON-form jest configs (turbo-test#31 item 1): a package.json "jest" block and
    // jest.config.json, with <rootDir>, a JSON rootDir and both setup keys
    let pj = tmp("jest-pkg-json");
    let ps = put(&pj, "setup.js", "");
    let pa = put(&pj, "after.js", "");
    put(
        &pj,
        "package.json",
        r#"{"name":"p","jest":{"setupFiles":["<rootDir>/setup.js"],"setupFilesAfterEnv":["./after.js"]}}"#,
    );
    assert_eq!(jest_setup_files(&pj), Some(vec![ps, pa]));
    let jj = tmp("jest-config-json");
    let js = put(&jj, "src/s.js", "");
    put(
        &jj,
        "jest.config.json",
        "{\n  \"rootDir\": \"src\",\n  \"setupFiles\": [\"<rootDir>/s.js\"]\n}\n",
    );
    assert_eq!(jest_setup_files(&jj), Some(vec![js]));
    // a package.json "jest" block with no setup keys is still a jest config (stop walking)
    let pe = tmp("jest-pkg-empty");
    put(
        &pe,
        "package.json",
        r#"{"name":"p","jest":{"testEnvironment":"node"}}"#,
    );
    assert_eq!(jest_setup_files(&pe), Some(vec![]));
    // package.json without jest / no config at all -> None (keep walking)
    let r = tmp("jest-none");
    assert_eq!(jest_setup_files(&r), None);
    put(&r, "package.json", r#"{ "name": "plain" }"#);
    assert_eq!(jest_setup_files(&r), None);
    // non-array values are ignored
    let t = tmp("jest-scalar");
    put(
        &t,
        "jest.config.js",
        "module.exports = { setupFiles: './x.js' }",
    );
    assert_eq!(jest_setup_files(&t), Some(vec![]));
    let u = tmp("jest-open");
    put(
        &u,
        "jest.config.mjs",
        "export default { setupFiles: ['./x.js' ",
    );
    assert_eq!(jest_setup_files(&u), Some(vec![]));
}

// ---- vi.hoisted / decorators ----------------------------------------------------------------

#[test]
fn extract_hoisted_statements() {
    let src = "import x from 'x';\nconst { a } = vi.hoisted(() => ({ a: fn(1) }));\nlet b = vi.hoisted(() => 2)\nfoo();";
    assert_eq!(
        extract_hoisted(src),
        vec![
            "const { a } = vi.hoisted(() => ({ a: fn(1) }));".to_string(),
            // no `;`: the statement ends at the call's closing paren
            "let b = vi.hoisted(() => 2)".to_string(),
        ]
    );
    // a bare call is hoisted too (vitest hoists every vi.hoisted)
    assert_eq!(
        extract_hoisted("vi.hoisted(() => 1);"),
        vec!["vi.hoisted(() => 1);".to_string()]
    );
    // turbo-test#31 item 7: a bare call after a declared one starts at the call, not at the
    // previous statement's `let` (which declared `b` twice in the prepass)
    assert_eq!(
        extract_hoisted("let b = vi.hoisted(() => 2)\nvi.hoisted(() => 3);"),
        vec![
            "let b = vi.hoisted(() => 2)".to_string(),
            "vi.hoisted(() => 3);".to_string()
        ]
    );
    // `await vi.hoisted(async ...)`, declared and bare
    assert_eq!(
        extract_hoisted("const { m } = await vi.hoisted(async () => ({ m: 1 }));\nawait vi.hoisted(async () => {});"),
        vec![
            "const { m } = await vi.hoisted(async () => ({ m: 1 }));".to_string(),
            "await vi.hoisted(async () => {});".to_string()
        ]
    );
    // a keyword that only ends an identifier (`outlet `) is not a declaration
    assert_eq!(
        extract_hoisted("outlet = vi.hoisted(() => 4);"),
        vec!["vi.hoisted(() => 4);".to_string()]
    );
    // unbalanced: runs to the end of the source
    assert_eq!(
        extract_hoisted("var h = vi.hoisted(() => (1"),
        vec!["var h = vi.hoisted(() => (1".to_string()]
    );
    // trailing whitespace then `;` is swallowed
    assert_eq!(
        extract_hoisted("const q = vi.hoisted(() => 1)  ;rest"),
        vec!["const q = vi.hoisted(() => 1)  ;".to_string()]
    );
    assert!(extract_hoisted("nothing").is_empty());
}

#[test]
fn file_has_decorator_detects_line_and_parameter_decorators() {
    assert!(file_has_decorator(
        "class A { constructor(@Inject() x) {} }"
    ));
    assert!(file_has_decorator("@Injectable()\nclass A {}"));
    assert!(file_has_decorator("class A {\n  @Prop() x;\n}"));
    assert!(file_has_decorator("@_private\nclass A {}"));
    assert!(file_has_decorator("@$dollar\nclass A {}"));
    assert!(!file_has_decorator("const email = 'a@b.c';"));
    assert!(!file_has_decorator("@ 1"));
    assert!(!file_has_decorator("@1"));
    assert!(!file_has_decorator("@"));
    assert!(!file_has_decorator(""));
}

#[test]
fn decorator_metadata_enabled_reads_nearest_tsconfig() {
    let d = tmp("decometa");
    put(
        &d,
        "on/tsconfig.json",
        "{ \"compilerOptions\": { \"emitDecoratorMetadata\" : true } }",
    );
    put(
        &d,
        "off/tsconfig.json",
        "{ \"compilerOptions\": {\n // \"emitDecoratorMetadata\": true\n \"emitDecoratorMetadata\": false } }",
    );
    assert!(decorator_metadata_enabled(&d.join("on/src/a.ts")));
    assert!(decorator_metadata_enabled(&d.join("on/b.ts")), "cached");
    assert!(!decorator_metadata_enabled(&d.join("off/a.ts")));
    let none = tmp("decometa-none");
    assert!(!decorator_metadata_enabled(&none.join("a.ts")));
}

// ---- vi.mock hoisting on esbuild CJS --------------------------------------------------------

const HOIST_IN: &str = r#""use strict";
var import_vitest = require("vitest");
var import_dep = require("./dep");
var import_helper = require("./helper");
var import_other = __toESM(require("./other"));
var mockFn = import_vitest.vi.fn();
import_vitest.vi.mock("./dep", () => ({
  dep: mockFn
}));
const spied = import_vitest.vi.mocked(import_dep.dep);
import_vitest.vi.mock("./other");
(0, import_vitest.it)("x", () => {
  (0, import_helper.h)(import_other.o);
});"#;

#[test]
fn hoist_mock_setup_moves_consumers_after_mocks_and_rebinds() {
    let out = hoist_mock_setup(HOIST_IN);
    let want = r#""use strict";
var import_vitest = require("vitest");
var import_dep = require("./dep");
var mockFn = import_vitest.vi.fn();
import_vitest.vi.mock("./dep", () => ({
  dep: mockFn
}));
import_vitest.vi.mock("./other");
import_dep = require("./dep");
import_other = __toESM(require("./other"));
const spied = import_vitest.vi.mocked(import_dep.dep);
var import_helper = require("./helper");
var import_other = __toESM(require("./other"));
(0, import_vitest.it)("x", () => {
  (0, import_helper.h)(import_other.o);
});"#;
    assert_eq!(out, want);
}

#[test]
fn hoist_mock_setup_keeps_requires_used_at_top_level() {
    let src = "var import_react = require(\"react\");\nvar import_a = require(\"./a\");\nclass X extends import_react.Component {}\nvi.mock(\"./a\");";
    // react is used at top level -> stays before; ./a is mocked and only consumed -> after,
    // and the mocked require is re-bound after registration.
    assert_eq!(
        hoist_mock_setup(src),
        "var import_react = require(\"react\");\nclass X extends import_react.Component {}\nvi.mock(\"./a\");\nimport_a = require(\"./a\");\nvar import_a = require(\"./a\");"
    );
}

#[test]
fn hoist_mock_setup_no_op_cases() {
    // no mock at all
    let plain = "var import_a = require(\"a\");\nfoo();";
    assert_eq!(hoist_mock_setup(plain), plain);
    // mock but no esbuild require block
    let no_req = "const a = require('a');\nvi.mock('a');";
    assert_eq!(hoist_mock_setup(no_req), no_req);
    // the only mock sits above/inside the require block
    let early = "vi.mock(\"a\");\nvar import_a = require(\"a\");";
    assert_eq!(hoist_mock_setup(early), early);
    // trailing newline is dropped by the line join (lines() semantics)
    let unbalanced = "var import_a = require(\"a\");\nvi.mock(\"a\", () => {";
    assert_eq!(
        hoist_mock_setup(unbalanced),
        "vi.mock(\"a\", () => {\nimport_a = require(\"a\");\nvar import_a = require(\"a\");"
    );
}

#[test]
fn extract_string_literal_and_require_spec() {
    assert_eq!(
        extract_first_string_literal("(x, \"a b\", 'c')").as_deref(),
        Some("a b")
    );
    assert_eq!(
        extract_first_string_literal("'single'").as_deref(),
        Some("single")
    );
    assert_eq!(
        extract_first_string_literal("'unterminated").as_deref(),
        Some("unterminated")
    );
    assert_eq!(extract_first_string_literal("no quotes"), None);
    assert_eq!(
        extract_require_spec("var import_x = __toESM(require(\"./x\"), 1);").as_deref(),
        Some("./x")
    );
    assert_eq!(extract_require_spec("var a = b;"), None);
    assert_eq!(extract_require_spec("require(dynamic)"), None);
    // turbo-test#42 item 4: an escaped quote does not end the specifier (esbuild prints
    // `./q"a'b.js` as "./q\"a'b.js"), and a template literal is a literal too
    assert_eq!(
        extract_require_spec(r#"var import_q = require("./q\"a'b.js");"#).as_deref(),
        Some("./q\"a'b.js")
    );
    assert_eq!(
        extract_first_string_literal("(`./q\"a'b.js`, () => 1)").as_deref(),
        Some("./q\"a'b.js")
    );
}

#[test]
fn js_string_literals_decode_their_escapes() {
    fn value(s: &str) -> Option<(String, Option<&str>)> {
        js_string_literal_at(s).map(|(v, len)| (v, len.map(|n| &s[..n])))
    }
    // the length runs through the closing quote; the other quotes are plain characters
    assert_eq!(
        value(r#""a'b`c" + x"#),
        Some(("a'b`c".into(), Some(r#""a'b`c""#)))
    );
    assert_eq!(value(r#"'a"b'"#), Some((r#"a"b"#.into(), Some(r#"'a"b'"#))));
    assert_eq!(value("`a'\"b`"), Some(("a'\"b".into(), Some("`a'\"b`"))));
    // escapes: quotes and backslash, the single-character escapes, hex and unicode
    assert_eq!(value(r#""\"\'\`\\""#).unwrap().0, "\"'`\\");
    assert_eq!(
        value(r#""\n\t\r\b\f\v\0""#).unwrap().0,
        "\n\t\r\u{8}\u{c}\u{b}\0"
    );
    assert_eq!(
        value(r#""\x41\u0042\u{43}\u{1F600}é""#).unwrap().0,
        "ABC\u{1F600}é"
    );
    // malformed hex / unicode escapes stand for their letter (JS rejects them)
    assert_eq!(
        value(r#""\xZ1\u12\u{}\u{110000}\x+1""#).unwrap().0,
        "xZ1u12u{}u{110000}x+1"
    );
    // any other escaped character is itself; line continuations are nothing (CRLF is one)
    assert_eq!(
        value("'\\q\\\n\\\r\n\\\r\\\u{2028}\\\u{2029}z'").unwrap().0,
        "qz"
    );
    // unterminated (also right after a backslash), and not a literal at all
    assert_eq!(value(r#""abc"#), Some(("abc".into(), None)));
    assert_eq!(value("'ab\\"), Some(("ab".into(), None)));
    assert_eq!(value("x'a'"), None);
    assert_eq!(value(""), None);
}

// ---- shared mock lets -------------------------------------------------------------------------

#[test]
fn shared_mock_lets_selects_factory_bound_mutables() {
    let src = r#"let capturedValue;
let capturedValue;
let hookValue = 1;
const capturedQueryFns = [];
const capturedThingsList: Array<() => void> = [
];
const shortArr = [];
const notAnArrayValue = {};
let short;
var notUsedInFactory = 1;
let shorthandName;
let neverAssigned1;
let [destructured] = x;
vi.mock('./x', () => { capturedValue = 1; capturedQueryFns.push(1); capturedThingsList.push(2); shortArr.push(1); short = 2; notAnArrayValue.a = 1; return { useX: () => hookValue + neverAssigned1, shorthandName }; });
it('t', () => { hookValue = 2; shorthandName = 3; });
"#;
    assert_eq!(
        shared_mock_lets(src),
        vec![
            "capturedValue".to_string(),
            "hookValue".to_string(),
            "capturedQueryFns".to_string(),
            "capturedThingsList".to_string(),
        ]
    );
    assert!(shared_mock_lets("let capturedValue = 1; capturedValue = 2;").is_empty());
}

#[test]
fn used_in_shorthand_cases() {
    assert!(used_in_shorthand("return { foo }", "foo"));
    assert!(used_in_shorthand("({foo, bar})", "foo"));
    assert!(used_in_shorthand("{ a,\n  foo\n}", "foo"));
    assert!(!used_in_shorthand("f(a, foo, b)", "foo"));
    assert!(!used_in_shorthand("[foo]", "foo"));
    assert!(!used_in_shorthand("foo", "foo"));
    assert!(!used_in_shorthand("{ foobar }", "foo"));
    assert!(!used_in_shorthand("{ afoo }", "foo"));
    assert!(!used_in_shorthand("{ foo: 1 }", "foo"));
}

#[test]
fn assigns_word_cases() {
    assert!(assigns_word("foo = 1", "foo"));
    assert!(assigns_word("x; foo\t= 1", "foo"));
    assert!(assigns_word("foo == 1; foo = 2", "foo"));
    assert!(!assigns_word("foo == 1", "foo"));
    assert!(!assigns_word("foo === 1", "foo"));
    assert!(!assigns_word("foo => 1", "foo"));
    assert!(!assigns_word("afoo = 1", "foo"));
    assert!(!assigns_word("foo2 = 1", "foo"));
    assert!(!assigns_word("foo += 1", "foo"));
    assert!(!assigns_word("x = foo", "foo"));
}

#[test]
fn strip_test_blocks_drops_callback_bodies() {
    assert_eq!(
        strip_test_blocks("it('a', () => { x(); });\nconst y = 1;"),
        "it;\nconst y = 1;"
    );
    assert_eq!(
        strip_test_blocks("item(1); description(2)"),
        "item(1); description(2)"
    );
    assert_eq!(
        strip_test_blocks("describe.only('x', fn); beforeEach(() => a())"),
        "describe.only; beforeEach"
    );
    assert_eq!(
        strip_test_blocks("(0, import_vitest.it)(\"x\", () => {}); keep"),
        "(0, import_vitest.it); keep"
    );
    assert_eq!(strip_test_blocks("it = 5"), "it = 5");
    assert_eq!(strip_test_blocks("it"), "it");
    assert_eq!(strip_test_blocks("test('a)', f) z"), "test z");
    assert_eq!(strip_test_blocks("test(`x${y})`) z"), "test z");
    assert_eq!(strip_test_blocks("test('a\\')', f) z"), "test z");
    assert_eq!(strip_test_blocks("test(() => { // )\n }) z"), "test z");
    assert_eq!(strip_test_blocks("test(/* ) */ 1) z"), "test z");
    assert_eq!(strip_test_blocks("é it(1) ü"), "é it ü");
    assert_eq!(strip_test_blocks("pre it(unclosed"), "pre it");
}

#[test]
fn contains_and_replace_word() {
    assert!(contains_word("a foo b", "foo"));
    assert!(contains_word("foobar foo", "foo"));
    assert!(!contains_word("foobar afoo", "foo"));
    assert!(!contains_word("", "foo"));

    let r = |t: &str| replace_word(t, "foo", "X");
    assert_eq!(r("a foo b"), "a X b");
    assert_eq!(r("foobar foo"), "foobar X");
    assert_eq!(r("obj.foo"), "obj.foo");
    assert_eq!(r("[...foo]"), "[...X]");
    assert_eq!(
        r("'foo' \"foo\" `foo ${foo}`"),
        "'foo' \"foo\" `foo ${foo}`"
    );
    assert_eq!(r("'a\\'foo' foo"), "'a\\'foo' X");
    assert_eq!(r("'é' foo"), "'é' X");
    assert_eq!(r("get foo() { return foo; }"), "get foo() { return X; }");
    assert_eq!(r("set  foo(v) {}"), "set  foo(v) {}");
    assert_eq!(r("target foo"), "target X");
    assert_eq!(r("getfoo"), "getfoo");
    assert_eq!(r("({ foo: 1 })"), "({ foo: 1 })");
    assert_eq!(r("é foo"), "é X");
    assert_eq!(r("x=foo"), "x=X");
    assert_eq!(r("foo"), "X");
    assert_eq!(r("'unterminated foo"), "'unterminated foo");
}

#[test]
fn rewrite_shared_lets_routes_through_global() {
    let code = "let capturedValue;\nvar other = 1;\nconst list = [];\nfn(() => { capturedValue = 42; list.push(other); });";
    let names = vec![
        "capturedValue".to_string(),
        "other".to_string(),
        "list".to_string(),
    ];
    assert_eq!(
        rewrite_shared_lets(code, &names),
        "globalThis.__ms = globalThis.__ms || {};\nglobalThis.__ms.capturedValue;\nglobalThis.__ms.other = 1;\nglobalThis.__ms.list = [];\nfn(() => { globalThis.__ms.capturedValue = 42; globalThis.__ms.list.push(globalThis.__ms.other); });"
    );
    assert_eq!(
        rewrite_shared_lets("x", &[]),
        "globalThis.__ms = globalThis.__ms || {};\nx"
    );
}

#[test]
fn postprocess_mr_cjs_rewrites_esbuild_helpers() {
    let src = "var __defProp = Object.defineProperty;\nvar __toESM = (mod) => mod;\nvar __toCommonJS = (mod) => __copyProps(__defProp({}, \"__esModule\", { value: true }), mod);\nrest";
    let out = postprocess_mr_cjs(src.to_string());
    assert!(postprocess_mr_cjs(
        "var __reExport = (target, mod, secondTarget) => (x);".to_string()
    )
    .contains("var __toESMStar = (m) => globalThis.__ttToESM(m); var __reExport = (target, mod, secondTarget) => (mod = __toESMStar(mod), x);"));
    let dep = postprocess_mr_dep_cjs(
        "var __toESM = (a) => a;\nvar __reExport = (target, mod, secondTarget) => (x);".to_string(),
        true,
    );
    assert!(dep
        .contains("var __toESM = (m, n) => globalThis.__ttToESMDep(m, n); var __toESM_unused = "));
    assert!(dep.contains("var __toESMStar = (m) => globalThis.__ttToESMDep(m, 1);"));
    let dep = postprocess_mr_dep_cjs(
        "var __reExport = (target, mod, secondTarget) => (x);".to_string(),
        false,
    );
    assert!(dep.contains("var __toESMStar = (m) => m;"));
    let native = dep_interop(crate::esm_cjs::PREAMBLE.to_string(), true);
    assert!(native.contains("var __toESM = (mod) => globalThis.__ttToESMDep(mod, 1);\n"));
    assert!(native.contains("var __toESMStar = (mod) => globalThis.__ttToESMDep(mod, 1);\n"));
    // a package's ES modules are marked "pkg" (turbo-test#159)
    assert!(native.contains("Symbol.for(\"turbo-test.esm\"), { value: \"pkg\" })"));
    assert!(postprocess_mr_dep_cjs(
        "var __toCommonJS = (mod) => __copyProps(__defProp({}, \"__esModule\", { value: true }), mod);".to_string(),
        false
    )
    .contains("Symbol.for(\"turbo-test.esm\"), { value: \"pkg\" })"));
    assert!(!postprocess_mr_cjs(
        "var __toCommonJS = (mod) => __copyProps(__defProp({}, \"__esModule\", { value: true }), mod);".to_string()
    )
    .contains("\"pkg\""));
    let native = dep_interop(crate::esm_cjs::PREAMBLE.to_string(), false);
    assert!(native.contains("var __toESM = (mod) => globalThis.__ttToESMDep(mod, 0);\n"));
    assert!(native.contains("var __toESMStar = (mod) => mod;\n"));
    assert!(out.contains(
        "var __defProp = (o, k, d) => Object.defineProperty(o, k, Object.assign({ configurable: true }, d));"
    ));
    assert!(out.contains("var __toESM = (m) => globalThis.__ttToESM(m); var __toESM_unused = "));
    assert!(out.contains(
        "var __toCommonJS = (mod) => __copyProps(__defProp(__defProp({}, \"__esModule\", { value: true }), Symbol.for(\"turbo-test.esm\"), { value: true }), mod);\n"
    ));
    assert!(out.contains("var __toESM_unused = (mod) => mod;"));
    assert!(out.ends_with("\nrest"));
    assert_eq!(postprocess_mr_cjs("plain".to_string()), "plain");
}

#[test]
fn feature_switches_default_on() {
    // the unit-test process never sets TURBO_NO_MR / TURBO_NO_E12
    assert!(mr_enabled());
    assert!(e12_enabled());
}

#[test]
fn esbuild_fallbacks_need_an_esbuild_project() {
    let d = tmp("fallback");
    let app = put(&d, "src/a.ts", "export const a = 1;");
    let dep = put(&d, "node_modules/x/index.js", "module.exports = 1;");
    assert_eq!(esbuild_cjs_fallback(&app, false), None);
    assert_eq!(esbuild_cjs_fallback(&dep, false), None);
    assert_eq!(esbuild_transform_cjs(&app, true), None);
    assert_eq!(esbuild_bundle_dep_cjs(&dep), None);
}

#[test]
fn read_transformed_passes_plain_js_through_on_the_esm_path() {
    let d = tmp("readtx");
    let f = put(&d, "a.js", "export const a = 1;\n");
    assert_eq!(
        read_transformed(&f, false, false).as_deref(),
        Some("export const a = 1;\n")
    );
    assert_eq!(read_transformed(&d.join("missing.js"), false, false), None);
}

#[test]
fn tsc_transform_esm_requires_typescript() {
    let d = tmp("tsc-none");
    let f = put(&d, "a.ts", "export const a: number = 1;");
    assert_eq!(tsc_transform_esm(&f, &d), None);
}

/// turbo-test#55: os.tmpdir() follows Node: TMPDIR, TMP, TEMP, then /tmp, trailing slash cut.
#[test]
fn os_tmpdir_follows_node() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    assert_eq!(node_tmpdir(env(&[("TMPDIR", "/a/b/")])), "/a/b");
    assert_eq!(node_tmpdir(env(&[("TMPDIR", ""), ("TMP", "/t")])), "/t");
    assert_eq!(node_tmpdir(env(&[("TEMP", "/e")])), "/e");
    assert_eq!(node_tmpdir(env(&[])), "/tmp");
    assert_eq!(node_tmpdir(env(&[("TMPDIR", "/")])), "/");
}

/// turbo-test#74: CI detection is std-env's `isCI` (std-env 4.3.0, which vitest 4.1.12 installs):
/// `CI` set to any non-empty value — `false` and `0` included — or the first provider whose
/// variable is non-empty, unless that provider marks itself as not CI. Every expectation here is
/// std-env's own answer (`scripts/check-ci-detection.mjs` checks the table against it).
#[test]
fn ci_detection_is_std_env_is_ci() {
    use crate::ci::is_ci;
    let env = |pairs: Vec<(&'static str, &'static str)>| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    // one row per std-env provider, each variable set alone
    let ci_rows = [
        "APPVEYOR",
        "AWS_APP_ID",
        "SYSTEM_TEAMFOUNDATIONCOLLECTIONURI",
        "INPUT_AZURE_STATIC_WEB_APPS_API_TOKEN",
        "AC_APPCIRCLE",
        "bamboo_planKey",
        "BITBUCKET_COMMIT",
        "BITRISE_IO",
        "BUDDY_WORKSPACE_ID",
        "BUILDKITE",
        "CIRCLECI",
        "CIRRUS_CI",
        "CF_PAGES",
        "WORKERS_CI",
        "K_SERVICE",
        "CLOUD_RUN_JOB",
        "CODEBUILD_BUILD_ARN",
        "CF_BUILD_ID",
        "DRONE",
        "DRONE_BUILD_EVENT",
        "DSARI",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "CI_MERGE_REQUEST_ID",
        "GO_PIPELINE_LABEL",
        "LAYERCI",
        "JENKINS_URL",
        "HUDSON_URL",
        "MAGNUM",
        "NETLIFY",
        "NEVERCODE",
        "RENDER",
        "SAILCI",
        "SEMAPHORE",
        "SCREWDRIVER",
        "SHIPPABLE",
        "TDDIUM",
        "STRIDER",
        "TEAMCITY_VERSION",
        "TRAVIS",
        "NOW_BUILDER",
        "APPCENTER_BUILD_ID",
        "STACKBLITZ",
        "STORMKIT",
        "CLEAVR",
        "ZEABUR",
        "CODESPHERE_APP_ID",
        "RAILWAY_PROJECT_ID",
        "RAILWAY_SERVICE_ID",
        "DENO_DEPLOY",
        "DENO_DEPLOYMENT_ID",
        "FIREBASE_APP_HOSTING",
        "EO_PAGES_CI",
    ];
    for k in ci_rows {
        assert!(is_ci(env(vec![(k, "1")])), "{k}=1 is CI");
        assert!(!is_ci(env(vec![(k, "")])), "{k}= (empty) is not CI");
    }
    // providers std-env names but does not count as CI (local / preview environments)
    let not_ci_rows = [
        "NETLIFY_LOCAL",
        "VERCEL",
        "VERCEL_ENV",
        "CODESANDBOX_SSE",
        "CODESANDBOX_HOST",
    ];
    for k in not_ci_rows {
        assert!(!is_ci(env(vec![(k, "1")])), "{k}=1 is not CI");
        assert!(is_ci(env(vec![(k, "1"), ("CI", "true")])), "{k}=1 CI=true");
    }
    // the first matching row decides
    assert!(is_ci(env(vec![("NETLIFY", "1"), ("NETLIFY_LOCAL", "1")])));
    assert!(is_ci(env(vec![("NOW_BUILDER", "1"), ("VERCEL", "1")])));
    assert!(!is_ci(env(vec![
        ("VERCEL", "1"),
        ("RAILWAY_PROJECT_ID", "1")
    ])));
    // `CI`: any non-empty value; an empty one falls through to the providers
    for v in ["true", "1", "false", "0", "yes"] {
        assert!(is_ci(env(vec![("CI", v)])), "CI={v}");
    }
    assert!(!is_ci(env(vec![("CI", "")])));
    assert!(is_ci(env(vec![("CI", ""), ("GITHUB_ACTIONS", "true")])));
    assert!(!is_ci(env(vec![])));
    // not in std-env's table; test environments do not count either
    for k in ["TF_BUILD", "BITBUCKET_BUILD_NUMBER", "BUILD_ID", "TEST"] {
        assert!(!is_ci(env(vec![(k, "1")])), "{k}=1 is not CI");
    }
    assert!(!is_ci(env(vec![("NODE_ENV", "test")])));
}

#[test]
fn source_order_exports_reorders_esbuilds_registration() {
    let code = "var x_exports = {};\n__export(x_exports, {\n  a: () => a,\n  \"b-c\": () => bc,\n  default: () => x_default,\n  z: () => z\n});\nrest\n});".to_string();
    let order = ["default", "b-c", "a"].map(String::from);
    assert_eq!(
        source_order_exports(code, &order),
        "var x_exports = {};\n__export(x_exports, {\n  default: () => x_default,\n  \"b-c\": () => bc,\n  a: () => a,\n  z: () => z\n});\nrest\n});"
    );
    // no registration, or not esbuild's: unchanged
    assert_eq!(source_order_exports("plain".to_string(), &order), "plain");
    let other = "f(x_exports, {\n  a: () => a\n});".to_string();
    assert_eq!(source_order_exports(other.clone(), &order), other);
    // a registration that does not close: unchanged
    let open = "__export(x_exports, {\n  b: () => b,\n  a: () => a".to_string();
    assert_eq!(source_order_exports(open.clone(), &order), open);
}

#[test]
fn config_string_lists_are_the_literal_strings_of_the_key() {
    let list = |s: &str| config_string_list(s, "snapshotSerializers");
    assert_eq!(
        list("test: { snapshotSerializers: ['./a.js', \"./b.js\", `./c.js`, ] }"),
        ["./a.js", "./b.js", "./c.js"]
    );
    // absent, not a literal list, or a list that does not close: none
    assert!(list("test: { setupFiles: ['./a.js'] }").is_empty());
    assert!(list("test: { snapshotSerializers: serializers }").is_empty());
    assert!(list("test: { snapshotSerializers: ['./a.js'").is_empty());
}

#[test]
#[ignore = "turbo-test#191: fails until the fix"]
fn a_serializers_variable_does_not_read_the_next_keys_list() {
    // vitest 4.1.12 takes the variable's value; the static reader takes no list (turbo-test#191)
    let list = |s: &str| config_string_list(s, "snapshotSerializers");
    assert_eq!(
        list("const s = [];\nexport default { test: { snapshotSerializers: s, setupFiles: ['./setup.js'] } };"),
        Vec::<String>::new()
    );
    assert_eq!(list("test: { snapshotSerializers:\n  ['./a.js'] }"), ["./a.js"]);
}
