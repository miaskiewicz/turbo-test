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
    for src in [
        "import.meta.glob(x)",
        "import.meta.glob('./*.ts', { query: x })",
        "import.meta.glob('./*.ts', { base: x })",
        "import.meta.glob('./*.ts', { query: { a: x } })",
        "import.meta.glob('./*.ts', { eager: x })",
        "import.meta.glob(['./*.ts', ...more])",
        "import.meta.glob()",
        "import.meta.glob('./*.ts', {}, 1)",
        "const g = import.meta.glob; g('./*.ts')",
        "other.glob('./*.ts'); import.meta.url",
        "import.meta.glob('./*.ts'",
        // options it does not read as literals: a spread, a computed key (vite evaluates these;
        // with unknown names vite fails the module)
        "import.meta.glob('./*.ts', { ...opts })",
        "import.meta.glob('./*.ts', { [k]: true })",
        "import.meta.glob('./*.ts', { query: { ...q } })",
        "import.meta.glob('./*.ts', { query: { [k]: 1 } })",
        // what vite rejects at transform (turbo-test#190: it fails at run time here): a base that
        // is not anchored, options that are not an object literal
        "import.meta.glob('./*.ts', { base: 'lib' })",
        "import.meta.glob('./*.ts', 5)",
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
               import.meta.glob('/lib/x/*.ts', { base: '@lib', as: 'url' });\n\
               import.meta.glob(['@lib/**/*.ts', '!@lib/x/m*.ts'], { query: 'q=1', import: 'n' });\n\
               import.meta.glob('~/src/*.ts', { query: '?raw' });\n\
               import.meta.glob('./*.ts', { as: 'raw', import: 'x', query: '' });";
    assert_eq!(
        expand(&file, src, &d.0, &aliases).unwrap(),
        "#!/usr/bin/env node\nimport * as __tt_glob_0_0 from \"../lib/x/l.ts?raw=true&n=1&s=a+b%26\";\
         ({\"./x/l.ts\": __tt_glob_0_0});\n\
         ({\"./x/l.ts\": () => import(\"../lib/x/l.ts?url\").then((m) => m[\"default\"])});\n\
         ({\"/lib/x/l.ts\": () => import(\"../lib/x/l.ts?q=1\").then((m) => m[\"n\"])});\n\
         ({\"/src/a.ts\": () => import(\"./a.ts?raw\")});\n\
         ({\"./a.ts\": () => import(\"./a.ts?raw\").then((m) => m[\"x\"])});"
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
