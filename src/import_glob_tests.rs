use super::*;

fn project(files: &[&str]) -> tempdir::Dir {
    tempdir::Dir::new(files)
}

mod tempdir {
    use std::path::PathBuf;
    pub struct Dir(pub PathBuf);
    impl Dir {
        pub fn new(files: &[&str]) -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let d = std::env::temp_dir()
                .join(format!("tt-import-glob-{}-{n}", std::process::id()))
                .canonicalize()
                .unwrap_or_else(|_| {
                    std::env::temp_dir().join(format!("tt-import-glob-{}-{n}", std::process::id()))
                });
            let _ = std::fs::remove_dir_all(&d);
            for f in files {
                let p = d.join(f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, "").unwrap();
            }
            Dir(d.canonicalize().unwrap())
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn eager_lazy_import_negation_and_line_count() {
    let d = project(&[
        "src/m.ts",
        "src/a.ts",
        "src/b.js",
        "src/.h.ts",
        "src/node_modules/x/i.ts",
        "src/deep/z.ts",
    ]);
    let file = d.0.join("src/m.ts");
    let src = "const a = import.meta.glob('./*.ts', {\n  eager: true,\n});\nconst b = import.meta.glob(['./**/*', '!./b.js'], { import: 'default' });\nconst c = (import.meta as any).glob(`./*.js`, { eager: true, import: 'n' });\n";
    let out = expand(&file, src, &d.0, &[]).unwrap();
    assert_eq!(
        out,
        "import * as __tt_glob_0_0 from \"./a.ts\";import * as __tt_glob_2_0 from \"./b.js\";\
         const a = ({\"./a.ts\": __tt_glob_0_0}\n\n);\n\
         const b = ({\"./a.ts\": () => import(\"./a.ts\").then((m) => m[\"default\"]), \"./deep/z.ts\": () => import(\"./deep/z.ts\").then((m) => m[\"default\"])});\n\
         const c = ({\"./b.js\": __tt_glob_2_0[\"n\"]});\n"
    );
    assert_eq!(out.lines().count(), src.lines().count());
}

#[test]
fn exhaustive_root_patterns_and_parent_dirs() {
    let d = project(&[
        "src/sub/m.ts",
        "src/app.ts",
        "src/.h.ts",
        "src/node_modules/i.ts",
        "lib/l.ts",
    ]);
    let file = d.0.join("src/sub/m.ts");
    let src = "import.meta.glob('../*.ts', { exhaustive: true }); import.meta.glob('/lib/*.ts'); import.meta.glob('../**/*.ts')";
    let out = expand(&file, src, &d.0, &[]).unwrap();
    assert_eq!(
        out,
        "({\"../.h.ts\": () => import(\"../.h.ts\"), \"../app.ts\": () => import(\"../app.ts\")}); \
         ({\"/lib/l.ts\": () => import(\"../../lib/l.ts\")}); \
         ({\"../app.ts\": () => import(\"../app.ts\")})"
    );
}

#[test]
fn calls_it_cannot_replace_are_left_alone() {
    let d = project(&["m.ts", "a.ts"]);
    let file = d.0.join("m.ts");
    // (what vite rejects fails the module instead: vites_option_evaluation_and_errors)
    for src in [
        "const g = import.meta.glob; g('./*.ts')",
        "other.glob('./*.ts'); import.meta.url",
        "import.meta.glob('./*.ts'",
        // options this transform does not evaluate (vite evaluates them; a call or a global)
        "import.meta.glob('./*.ts', { eager: f() })",
        "import.meta.glob('./*.ts', { query: { a: Number } })",
        "import.meta.glob('./*.ts', { ...Object })",
    ] {
        assert_eq!(expand(&file, src, &d.0, &[]), None, "{src}");
    }
    assert_eq!(expand(&file, "no meta here", &d.0, &[]), None);
    assert_eq!(
        expand(
            &file,
            "import.meta.glob('./nope/*.ts', { 'eager': true })",
            &d.0,
            &[]
        )
        .unwrap(),
        "({})"
    );
}

#[test]
fn base_query_as_aliases_and_a_hashbang() {
    let d = project(&["src/m.ts", "src/a.ts", "lib/x/l.ts", "lib/x/.d.ts"]);
    let file = d.0.join("src/m.ts");
    let lib = d.0.join("lib").to_string_lossy().into_owned();
    let aliases = [
        ("@lib".to_string(), lib),
        ("~".to_string(), ".".to_string()),
    ];
    let src = "#!/usr/bin/env node\nimport.meta.glob('./x/*.ts', { base: '../lib', eager: true, query: { raw: true, n: 1, s: 'a b&' } });\n\
               import.meta.glob('/lib/x/*.ts', { base: '/lib', as: 'url' });\n\
               import.meta.glob(['@lib/**/*.ts', '!@lib/x/m*.ts'], { query: 'q=1', import: 'n' });\n\
               import.meta.glob('~/src/*.ts', { query: '?raw' });\n\
               import.meta.glob('./*.ts', { as: 'raw', import: '*', query: '' });";
    assert_eq!(
        expand(&file, src, &d.0, &aliases).unwrap(),
        "#!/usr/bin/env node\nimport * as __tt_glob_0_0 from \"../lib/x/l.ts?raw=true&n=1&s=a+b%26\";\
         ({\"./x/l.ts\": __tt_glob_0_0});\n\
         ({\"./x/l.ts\": () => import(\"../lib/x/l.ts?url\").then((m) => m[\"default\"])});\n\
         ({\"/lib/x/l.ts\": () => import(\"../lib/x/l.ts?q=1\").then((m) => m[\"n\"])});\n\
         ({\"/src/a.ts\": () => import(\"./a.ts?raw\")});\n\
         ({\"./a.ts\": () => import(\"./a.ts?raw\")});"
    );
    // a quoted query key is form-encoded as an identifier one is
    assert_eq!(
        expand(
            &file,
            "import.meta.glob('./*.ts', { query: { 'a b': 1 } })",
            &d.0,
            &[]
        )
        .unwrap(),
        "({\"./a.ts\": () => import(\"./a.ts?a+b=1\")})"
    );
    // an alias outside the root: absolute keys
    let out = std::env::temp_dir();
    assert_eq!(
        expand(
            &file,
            "import.meta.glob('@lib/x/*.ts')",
            &out.join("elsewhere"),
            &aliases[..1]
        )
        .unwrap(),
        format!(
            "({{\"{}/x/l.ts\": () => import(\"../lib/x/l.ts\")}})",
            aliases[0].1
        )
    );
}

#[test]
fn an_unanchored_pattern_fails_the_module() {
    let d = project(&["m.ts"]);
    let file = d.0.join("m.ts");
    let msg = |p: &str| {
        format!("throw new Error(\"Invalid glob: \\\"{p}\\\" (resolved: \\\"{p}\\\"). It must start with '/' or './'\");")
    };
    assert_eq!(
        expand(&file, "a;\nimport.meta.glob('@/*.ts');", &d.0, &[]).unwrap(),
        format!("{}a;\nimport.meta.glob('@/*.ts');", msg("@/*.ts"))
    );
    assert_eq!(
        expand(
            &file,
            "#!/bin/x\nimport.meta.glob(['./*.ts', '!x/*.ts']);",
            &d.0,
            &[]
        )
        .unwrap(),
        format!(
            "#!/bin/x\n{}import.meta.glob(['./*.ts', '!x/*.ts']);",
            msg("x/*.ts")
        )
    );
    // an alias anchors it
    let aliases = [("@".to_string(), d.0.to_string_lossy().into_owned())];
    assert_eq!(
        expand(&file, "import.meta.glob('@/*.ts')", &d.0, &aliases).unwrap(),
        "({})"
    );
}

#[test]
fn relative_specs() {
    assert_eq!(
        relative_spec(Path::new("/a/b"), Path::new("/a/b/c.ts")),
        "./c.ts"
    );
    assert_eq!(
        relative_spec(Path::new("/a/b"), Path::new("/a/x/c.ts")),
        "../x/c.ts"
    );
    assert_eq!(
        relative_spec(Path::new("/a/b/../b"), Path::new("/c.ts")),
        "../../c.ts"
    );
}

#[test]
fn a_cast_import_meta_and_overlapping_patterns() {
    // vitest 4.1.12: `(import.meta satisfies any).glob`, `import.meta!.glob` and
    // `(<any>import.meta).glob` expand as `import.meta.glob`; a module two patterns match is
    // listed once
    let d = project(&["m/a.ts", "m/b.ts", "t.ts"]);
    let file = d.0.join("t.ts");
    let lazy =
        "({\"./m/a.ts\": () => import(\"./m/a.ts\"), \"./m/b.ts\": () => import(\"./m/b.ts\")})";
    for src in [
        "(import.meta satisfies any).glob('./m/*.ts')",
        "import.meta!.glob('./m/*.ts')",
        "(<any>import.meta).glob('./m/*.ts')",
        "import.meta.glob(['./m/b.ts', './m/*.ts'])",
    ] {
        assert_eq!(expand(&file, src, &d.0, &[]).unwrap(), lazy, "{src}");
    }
}

#[test]
fn normal_folds_dot_segments() {
    assert_eq!(normal(Path::new("./a/./b/../c")), Path::new("a/c"));
    assert_eq!(normal(Path::new("/a/../../b")), Path::new("/b"));
}

/// turbo-test#190: vite evaluates the options literal (spreads, computed keys, `!0`, `+`) and
/// fails the module at transform on what its parseImportGlob / parseGlobOptions reject. Rows
/// recorded from vitest 4.1.12 (vite 8.3.5) on the same project; turbo-test left these calls
/// alone (vitest's run-time "statically replaced" error) or expanded what vite rejects.
#[test]
fn vites_option_evaluation_and_errors() {
    let d = project(&["m.ts", "a.ts", "b.ts"]);
    let file = d.0.join("m.ts");
    let ab = |a: &str, b: &str| format!("({{\"./a.ts\": {a}, \"./b.ts\": {b}}})");
    let eager = format!(
        "import * as __tt_glob_0_0 from \"./a.ts\";import * as __tt_glob_0_1 from \"./b.ts\";{}",
        ab("__tt_glob_0_0", "__tt_glob_0_1")
    );
    let lazy = |q: &str| {
        ab(
            &format!("() => import(\"./a.ts{q}\")"),
            &format!("() => import(\"./b.ts{q}\")"),
        )
    };
    let expanded: [(&str, String); 14] = [
        (
            "import.meta.glob('./*.ts', { ...{ eager: true } })",
            eager.clone(),
        ),
        ("import.meta.glob('./*.ts', { eager: !0 })", eager.clone()),
        (
            "import.meta.glob('./*.ts', { eager: false, ...{ eager: true } })",
            eager.clone(),
        ),
        (
            "import.meta.glob('./*.ts', { ...{ ...{ eager: true } }, ...null, ...5 })",
            eager.clone(),
        ),
        (
            "import.meta.glob('./*.ts', { ['eag' + 'er']: true } as any)",
            eager.clone(),
        ),
        (
            "import.meta.glob('./*.ts', { query: { ...{ a: 1 }, b: '', c: false, d: -1.5 } })",
            lazy("?a=1&b&c=false&d=-1.5"),
        ),
        (
            "import.meta.glob('./*.ts', { query: { a: 1, ...{ a: 2 } } })",
            lazy("?a=2"),
        ),
        (
            "import.meta.glob('./*.ts', { caseSensitive: true, base: '', exhaustive: false })",
            lazy(""),
        ),
        ("import.meta.glob('./*.ts', { as: 'foo' })", lazy("?foo")),
        (
            "import.meta.glob('./*.ts', { as: 'raw', query: '' })",
            ab(
                "() => import(\"./a.ts?raw\").then((m) => m[\"default\"])",
                "() => import(\"./b.ts?raw\").then((m) => m[\"default\"])",
            ),
        ),
        ("import.meta.glob('./*.ts', { import: '*' })", lazy("")),
        (
            "import.meta.glob('./*.ts', { import: '', query: `x` })",
            lazy("?x"),
        ),
        ("import.meta.glob([])", "({})".to_string()),
        ("import.meta.glob(['./a.ts', , './b.ts'])", lazy("")),
    ];
    for (src, want) in expanded {
        assert_eq!(
            expand(&file, src, &d.0, &[]).as_deref(),
            Some(want.as_str()),
            "{src}"
        );
    }
    let syntax = |m: &str| format!("Invalid glob import syntax: {m}");
    let errors: [(&str, String); 24] = [
        ("import.meta.glob('./*.ts', { base: 'lib' })", "Option \"base\" must start with '/', './' or '../', but got \"lib\"".into()),
        ("import.meta.glob('sub/*.ts', { base: 'lib' })", "Option \"base\" must start with '/', './' or '../', but got \"lib\"".into()),
        ("import.meta.glob('./*.ts', { base: '!x' })", "Option \"base\" cannot start with \"!\"".into()),
        ("import.meta.glob('./*.ts', 5)", syntax("Expected the second argument to be an object literal, but got \"Literal\"")),
        ("import.meta.glob('./*.ts', 5 as any)", syntax("Expected the second argument to be an object literal, but got \"Literal\"")),
        ("import.meta.glob('./*.ts', p)", syntax("Expected the second argument to be an object literal, but got \"Identifier\"")),
        ("import.meta.glob('./*.ts', ...p)", syntax("Expected the second argument to be an object literal, but got \"SpreadElement\"")),
        ("import.meta.glob('./*.ts', {}, {})", syntax("Expected 1-2 arguments, but got 3")),
        ("import.meta.glob()", syntax("Expected 1-2 arguments, but got 0")),
        ("import.meta.glob(5)", syntax("Expected glob to be a string, but got \"number\"")),
        ("import.meta.glob([null])", syntax("Expected glob to be a string, but got \"object\"")),
        ("import.meta.glob(p)", syntax("Could only use literals")),
        ("import.meta.glob(['./*.ts', ...more])", syntax("Could only use literals")),
        ("import.meta.glob(`./${p}`)", syntax("Expected glob to be a string, but got dynamic template literal")),
        ("import.meta.glob('./*.ts', { foo: 1 })", "Unknown glob option \"foo\"".into()),
        ("import.meta.glob('./*.ts', { eager: true, foo: 1, base: 'x' })", "Unknown glob option \"foo\"".into()),
        ("import.meta.glob('./*.ts', { eager: 'yes' })", "Expected glob option \"eager\" to be of type boolean, but got string".into()),
        ("import.meta.glob('./*.ts', { eager: undefined })", "Expected glob option \"eager\" to be of type boolean, but got undefined".into()),
        ("import.meta.glob('./*.ts', { eager: null })", "Expected glob option \"eager\" to be of type boolean, but got object".into()),
        ("import.meta.glob('./*.ts', { query: 5 })", "Expected glob option \"query\" to be of type object or string, but got number".into()),
        ("import.meta.glob('./*.ts', { query: { a: {} } })", "Expected glob option \"query.a\" to be of type string, number, or boolean, but got object".into()),
        ("import.meta.glob('./*.ts', { as: 'raw', query: '?x' })", "Options \"as\" and \"query\" cannot be used together".into()),
        ("import.meta.glob('./*.ts', { as: 'raw', import: 'x' })", "Option \"import\" can only be \"default\" or \"*\" when \"as\" is \"raw\", but got \"x\"".into()),
        ("import.meta.glob('./*.ts', { eager: isEager })", "Vite is unable to parse the glob options as the value is not static".into()),
    ];
    for (src, message) in errors {
        let message = serde_json::to_string(&message).unwrap();
        assert_eq!(
            expand(&file, src, &d.0, &[]),
            Some(format!("throw new Error({message});{src}")),
            "{src}"
        );
    }
    // the options' unknown names fail like a free variable; an option error wins over an
    // invalid pattern of an earlier call
    for src in [
        "import.meta.glob('./*.ts', { ...p })",
        "import.meta.glob('./*.ts', { [p]: 1 })",
        "import.meta.glob('./*.ts', { query: { a: q } })",
        "import.meta.glob('./*.ts', { eager: Foo })",
        "import.meta.glob('x/*.ts'); import.meta.glob('./*.ts', { eager })",
    ] {
        assert_eq!(
            expand(&file, src, &d.0, &[]),
            Some(format!("throw new Error(\"Vite is unable to parse the glob options as the value is not static\");{src}")),
            "{src}"
        );
    }
    // what this transform does not evaluate (calls, members, Node's globals) or does not take
    // (`caseSensitive: false`) is still left alone
    for src in [
        "import.meta.glob('./*.ts', { eager: Boolean(1) })",
        "import.meta.glob('./*.ts', { eager: Boolean })",
        "import.meta.glob('./*.ts', { eager: globalThis.x })",
        "import.meta.glob('./*.ts', { caseSensitive: false })",
    ] {
        assert_eq!(expand(&file, src, &d.0, &[]), None, "{src}");
    }
}
