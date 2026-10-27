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
        "import.meta.glob('@/*.ts')",
        "import.meta.glob(['./*.ts', ...more])",
        "import.meta.glob()",
        "import.meta.glob('./*.ts', {}, 1)",
        "const g = import.meta.glob; g('./*.ts')",
        "other.glob('./*.ts'); import.meta.url",
        "import.meta.glob('./*.ts'",
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
