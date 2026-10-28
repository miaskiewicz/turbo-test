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
        // options this transform does not evaluate (vite evaluates them; a global's value)
        "import.meta.glob('./*.ts', { query: { a: [1].length } })",
        "import.meta.glob('./*.ts', { eager: a?.b })",
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
        "import.meta.glob('./*.ts', { eager: [].length })",
        "import.meta.glob('./*.ts', { eager: 1 & 1 })",
    ] {
        assert_eq!(expand(&file, src, &d.0, &[]), None, "{src}");
    }
}

/// turbo-test#197: a call in the options fails the module as vite 4.1.12 does. vite cuts the
/// options at the end of their first call and fails its own parse; the closer named is the one of
/// the object or array the call sits in. vite's message goes on with a code frame of esbuild's
/// reprint of the cut text (`1: import.meta.glob("./m/*.ts", { eager: Boolean(1)` and a caret),
/// which this transform does not reproduce: the message here is its first two lines only.
#[test]
fn a_call_in_the_options_fails_the_parse_like_vite() {
    let d = project(&["t.ts", "a.ts"]);
    let file = d.0.join("t.ts");
    let rows = [
        // rows recorded from vitest 4.1.12
        ("import.meta.glob('./*.ts', { eager: Boolean(1) })", '}'),
        ("import.meta.glob('./*.ts', { eager: f() })", '}'),
        ("import.meta.glob('./*.ts', { query: { a: f() } })", '}'),
        ("import.meta.glob('./*.ts', { eager: f(g()) })", '}'),
        ("import.meta.glob('./*.ts', { eager: [f()] })", ']'),
        ("import.meta.glob('./*.ts', { eager: f(1, 2) })", '}'),
        // the parse fails before any name is evaluated
        (
            "import.meta.glob('./*.ts', { eager: isEager, query: { a: [1, f()] } })",
            ']',
        ),
        ("import.meta.glob('./*.ts', { [f()]: 1 })", ']'),
        ("import.meta.glob('./*.ts', { eager: `${f()}` })", '}'),
    ];
    for (src, closer) in rows {
        let message = format!("Parse failed with 1 error:\nExpected `{closer}` but found `EOF`");
        let message = serde_json::to_string(&message).unwrap();
        assert_eq!(
            expand(&file, src, &d.0, &[]),
            Some(format!("throw new Error({message});{src}")),
            "{src}"
        );
    }
}

/// `f` of the expression `src` (parsed as TypeScript, parenthesized: no directive).
fn with_expr<T>(src: &str, f: impl FnOnce(&Expression) -> T) -> T {
    let allocator = Allocator::default();
    let code = format!("({src});");
    let parsed = Parser::new(&allocator, &code, SourceType::ts()).parse();
    assert!(parsed.errors.is_empty(), "{src}");
    match &parsed.program.body[0] {
        oxc_ast::ast::Statement::ExpressionStatement(s) => f(&s.expression),
        _ => panic!("{src}"),
    }
}

#[test]
fn option_values_evaluate_as_js() {
    use Value::*;
    let ev = |src: &str| with_expr(src, eval);
    let obj =
        |kv: &[(&str, Value)]| Object(kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect());
    for (src, want) in [
        ("undefined", Ok(Undefined)),
        ("process", Ok(Undefined)),
        ("+NaN", Ok(Num(f64::NAN))),
        ("-Infinity", Ok(Num(f64::NEG_INFINITY))),
        ("+2", Ok(Num(2.0))),
        ("1 + 2", Ok(Num(3.0))),
        ("'a' + 1", Ok(Str("a1".into()))),
        ("1.5 + 'a'", Ok(Str("1.5a".into()))),
        ("`t`", Ok(Str("t".into()))),
        ("void x", Err(Eval::NotStatic)),
        ("void 0", Ok(Undefined)),
        ("!''", Ok(Bool(true))),
        ("!'a'", Ok(Bool(false))),
        ("!0", Ok(Bool(true))),
        ("!NaN", Ok(Bool(true))),
        ("!-1", Ok(Bool(false))),
        ("!null", Ok(Bool(true))),
        ("!{}", Ok(Bool(false))),
        ("!true", Ok(Bool(false))),
        (
            "[1, , 'b'] as any",
            Ok(obj(&[("0", Num(1.0)), ("2", Str("b".into()))])),
        ),
        (
            "({ b: 1, 2: 'x', a: 2, 1: 'y', b: 3 })",
            Ok(obj(&[
                ("b", Num(3.0)),
                ("2", Str("x".into())),
                ("a", Num(2.0)),
                ("1", Str("y".into())),
            ])),
        ),
        (
            "({ ...'ab', ...true, ...undefined })",
            Ok(obj(&[("0", Str("a".into())), ("1", Str("b".into()))])),
        ),
        (
            "({ ...{ 2: 0, 1: 0 } })",
            Ok(obj(&[("1", Num(0.0)), ("2", Num(0.0))])),
        ),
        (
            "({ [1.5]: 0, [null]: 0, [undefined]: 0, [true]: 0, [{}]: 0, 'q': 0 })",
            Ok(obj(&[
                ("1.5", Num(0.0)),
                ("null", Num(0.0)),
                ("undefined", Num(0.0)),
                ("true", Num(0.0)),
                ("[object Object]", Num(0.0)),
                ("q", Num(0.0)),
            ])),
        ),
        (
            "({ ['__proto__']: 1 })",
            Ok(obj(&[("__proto__", Num(1.0))])),
        ),
        // what this does not model
        ("({ ...'é' })", Err(Eval::Unsupported)),
        ("({ __proto__: {} })", Err(Eval::Unsupported)),
        ("({ '__proto__': {} })", Err(Eval::Unsupported)),
        ("({ get a() { return 1; } })", Err(Eval::Unsupported)),
        ("({ a() {} })", Err(Eval::Unsupported)),
        ("[...x]", Err(Eval::Unsupported)),
        ("-'1'", Err(Eval::Unsupported)),
        ("+true", Err(Eval::Unsupported)),
        ("typeof x", Ok(Str("undefined".into()))),
        ("typeof (x)", Ok(Str("undefined".into()))),
        ("typeof x.y", Err(Eval::NotStatic)),
        ("typeof 1", Ok(Str("number".into()))),
        ("typeof Math", Ok(Str("object".into()))),
        ("typeof Map", Ok(Str("function".into()))),
        ("null + 1", Err(Eval::Unsupported)),
        ("1 - 1", Ok(Num(0.0))),
        ("`${1}-${'x'}-${null}`", Ok(Str("1-x-null".into()))),
        ("`${Math}`", Err(Eval::Unsupported)),
        ("`${x}`", Err(Eval::NotStatic)),
        // members (turbo-test#195)
        ("({ a: 1 }).a", Ok(Num(1.0))),
        ("({ a: 1 })['b']", Ok(Undefined)),
        ("({ a: 1 }).toString", Err(Eval::Unsupported)),
        ("[1].length", Err(Eval::Unsupported)),
        ("({ a: 1 })[Math]", Err(Eval::Unsupported)),
        ("'ab'.length", Ok(Num(2.0))),
        ("'é'.length", Ok(Num(1.0))),
        ("'ab'[1]", Ok(Str("b".into()))),
        ("'ab'[2]", Ok(Undefined)),
        ("'ab'['01']", Err(Eval::Unsupported)),
        ("'é'[0]", Err(Eval::Unsupported)),
        ("'ab'.at", Err(Eval::Unsupported)),
        ("(1).x", Err(Eval::Unsupported)),
        ("undefined.x", Err(Eval::NotStatic)),
        ("null[0]", Err(Eval::NotStatic)),
        ("globalThis.Map", Ok(Global("Map".into()))),
        ("globalThis.process", Ok(Global("process".into()))),
        ("globalThis.nope", Ok(Undefined)),
        ("globalThis.localStorage", Ok(Undefined)),
        (
            "Math.E + Math.LN2 + Math.LN10 + Math.LOG2E + Math.LOG10E + Math.SQRT2 + Math.SQRT1_2",
            Ok(Num(std::f64::consts::E
                + std::f64::consts::LN_2
                + std::f64::consts::LN_10
                + std::f64::consts::LOG2_E
                + std::f64::consts::LOG10_E
                + std::f64::consts::SQRT_2
                + std::f64::consts::FRAC_1_SQRT_2)),
        ),
        ("Number.MIN_SAFE_INTEGER", Ok(Num(-9007199254740991.0))),
        ("Number.EPSILON", Ok(Num(f64::EPSILON))),
        ("Number.MAX_VALUE", Ok(Num(f64::MAX))),
        ("Number.MIN_VALUE", Ok(Num(5e-324))),
        ("Number.POSITIVE_INFINITY", Ok(Num(f64::INFINITY))),
        ("Number.NEGATIVE_INFINITY", Ok(Num(f64::NEG_INFINITY))),
        ("Number.NaN", Ok(Num(f64::NAN))),
        ("Number.parseInt", Err(Eval::Unsupported)),
        ("Math.max", Err(Eval::Unsupported)),
        // operators
        ("7 % 4 / 2 * 3 - 1", Ok(Num(3.5))),
        ("1 < 2", Ok(Bool(true))),
        ("2 <= 2", Ok(Bool(true))),
        ("1 > 2", Ok(Bool(false))),
        ("1 >= NaN", Ok(Bool(false))),
        ("'B' < 'a'", Ok(Bool(true))),
        ("'b' >= 'a'", Ok(Bool(true))),
        ("'1' < 2", Err(Eval::Unsupported)),
        ("1 === 1", Ok(Bool(true))),
        ("NaN === NaN", Ok(Bool(false))),
        ("'1' === 1", Ok(Bool(false))),
        ("'a' !== 'a'", Ok(Bool(false))),
        ("({}) === ({})", Err(Eval::Unsupported)),
        ("1 === Map", Err(Eval::Unsupported)),
        ("null == undefined", Ok(Bool(true))),
        ("null != 0", Ok(Bool(true))),
        ("0 != undefined", Ok(Bool(true))),
        ("true == true", Ok(Bool(true))),
        ("'a' != 'b'", Ok(Bool(true))),
        ("'1' == 1", Err(Eval::Unsupported)),
        ("Map == Map", Err(Eval::Unsupported)),
        ("1 << 1", Err(Eval::Unsupported)),
        ("0 && x", Ok(Num(0.0))),
        ("1 && 'r'", Ok(Str("r".into()))),
        ("'l' || x", Ok(Str("l".into()))),
        ("'' || 2", Ok(Num(2.0))),
        ("0 ?? x", Ok(Num(0.0))),
        ("null ?? 'd'", Ok(Str("d".into()))),
        ("1 ? 'y' : x", Ok(Str("y".into()))),
        ("0 ? x : 'n'", Ok(Str("n".into()))),
        ("x ? 1 : 2", Err(Eval::NotStatic)),
        ("({ [Math]: 1 })", Err(Eval::Unsupported)),
        ("({ ...Map })", Ok(obj(&[]))),
        ("!Map", Ok(Bool(false))),
        ("Math", Ok(Global("Math".into()))),
        ("Mathx", Err(Eval::NotStatic)),
        ("1n", Err(Eval::Unsupported)),
    ] {
        let got = ev(src);
        match (&got, &want) {
            (Ok(Num(a)), Ok(Num(b))) if a.is_nan() => assert!(b.is_nan(), "{src}"),
            _ => assert_eq!(got, want, "{src}"),
        }
    }
    // typeof, String() and truthiness of each kind
    let all = [
        Undefined,
        Null,
        Bool(true),
        Num(-0.0),
        Str(String::new()),
        obj(&[]),
    ];
    let types: Vec<&str> = all.iter().map(Value::type_of).collect();
    assert_eq!(
        types,
        [
            "undefined",
            "object",
            "boolean",
            "number",
            "string",
            "object"
        ]
    );
    let strings: Vec<String> = all.iter().map(Value::to_js_string).collect();
    assert_eq!(
        strings,
        ["undefined", "null", "true", "0", "", "[object Object]"]
    );
    let truthy: Vec<bool> = all.iter().map(Value::truthy).collect();
    assert_eq!(truthy, [false, false, true, false, false, true]);
    assert!(Str("a".into()).truthy() && Num(2.0).truthy() && !Bool(false).truthy());
}

#[test]
fn numbers_print_as_js() {
    for (n, want) in [
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (-0.0, "0"),
        (123.0, "123"),
        (-1.5, "-1.5"),
        (0.1, "0.1"),
        (1e-6, "0.000001"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (-1.5e300, "-1.5e+300"),
        (1.5e-7, "1.5e-7"),
    ] {
        assert_eq!(js_number(n), want, "{n}");
    }
}

#[test]
fn argument_node_types_and_glob_literals() {
    for (src, want) in [
        ("1", "Literal"),
        ("/x/", "Literal"),
        ("1n", "Literal"),
        ("(null as any)", "Literal"),
        ("`x`", "TemplateLiteral"),
        ("x", "Identifier"),
        ("import.meta", "MetaProperty"),
        ("[]", "ArrayExpression"),
        ("() => 1", "ArrowFunctionExpression"),
        ("x = 1", "AssignmentExpression"),
        ("async () => await x", "ArrowFunctionExpression"),
        ("await x", "AwaitExpression"),
        ("1 + 1", "BinaryExpression"),
        ("f()", "CallExpression"),
        ("a?.b", "ChainExpression"),
        ("(class {})", "ClassExpression"),
        ("a ? b : c", "ConditionalExpression"),
        ("(function () {})", "FunctionExpression"),
        ("import('x')", "ImportExpression"),
        ("a || b", "LogicalExpression"),
        ("new X()", "NewExpression"),
        ("(a, b)", "SequenceExpression"),
        ("t`x`", "TaggedTemplateExpression"),
        ("this", "ThisExpression"),
        ("-x", "UnaryExpression"),
        ("x++", "UpdateExpression"),
        ("a.b", "MemberExpression"),
        ("a[0]", "MemberExpression"),
        ("({})", "ObjectExpression"),
    ] {
        let got = with_expr(src, estree_type);
        if want == "ObjectExpression" {
            // parse_call takes an object literal before naming it
            assert_eq!(got, "Expression", "{src}");
        } else {
            assert_eq!(got, want, "{src}");
        }
    }
    let lit = |src: &str| with_expr(src, glob_literal);
    assert_eq!(lit("'./a'"), Ok("./a".to_string()));
    assert_eq!(lit("`./b`"), Ok("./b".to_string()));
    assert_eq!(
        lit("true"),
        Err("Expected glob to be a string, but got \"boolean\"".to_string())
    );
    assert_eq!(
        lit("1n"),
        Err("Expected glob to be a string, but got \"bigint\"".to_string())
    );
    assert_eq!(
        lit("/x/"),
        Err("Expected glob to be a string, but got \"object\"".to_string())
    );
    assert_eq!(lit("f()"), Err("Could only use literals".to_string()));
    // a spread first argument is no literal (vite's validateLiteral)
    let d = project(&["m.ts"]);
    // second arguments only a generator, a class or JSX can hold
    let second = |m: &str| {
        format!("throw new Error(\"Invalid glob import syntax: Expected the second argument to be an object literal, but got \\\"{m}\\\"\");")
    };
    for (file, src, ty) in [
        (
            "m.ts",
            "function* g() { import.meta.glob('./*.ts', yield); }",
            "YieldExpression",
        ),
        (
            "m.ts",
            "class C { #x = 1; m(o: any) { import.meta.glob('./*.ts', #x in o); } }",
            "BinaryExpression",
        ),
        ("m.tsx", "import.meta.glob('./*.ts', <a />);", "JSXElement"),
        ("m.tsx", "import.meta.glob('./*.ts', <></>);", "JSXFragment"),
    ] {
        assert_eq!(
            expand(&d.0.join(file), src, &d.0, &[]),
            Some(format!("{}{src}", second(ty))),
            "{src}"
        );
    }
    assert_eq!(
        expand(&d.0.join("m.ts"), "import.meta.glob(...p)", &d.0, &[]),
        Some("throw new Error(\"Invalid glob import syntax: Could only use literals\");import.meta.glob(...p)".to_string())
    );
}

/// turbo-test#195: `caseSensitive: false` matches case-insensitively (keys keep the files'
/// case), and the options' member reads, operators and `typeof` evaluate as vite's evalValue
/// does. Rows recorded from vitest 4.1.12 (vite 8.3.5); turbo-test left these calls alone.
#[test]
fn case_insensitive_globs_and_evaluated_members() {
    let d = project(&["t.ts", "m/a.ts", "m/B.ts"]);
    let file = d.0.join("t.ts");
    let lazy = |files: &[&str], q: &str| {
        let e: Vec<String> = files
            .iter()
            .map(|f| format!("\"./m/{f}\": () => import(\"./m/{f}{q}\")"))
            .collect();
        format!("({{{}}})", e.join(", "))
    };
    let eager = |pick: &str| {
        format!(
            "import * as __tt_glob_0_0 from \"./m/B.ts\";import * as __tt_glob_0_1 from \"./m/a.ts\";\
             ({{\"./m/B.ts\": __tt_glob_0_0[\"{pick}\"], \"./m/a.ts\": __tt_glob_0_1[\"{pick}\"]}})"
        )
    };
    let expanded: [(&str, String); 8] = [
        ("import.meta.glob('./m/*.ts', { caseSensitive: false })", lazy(&["B.ts", "a.ts"], "")),
        ("import.meta.glob('./m/*.TS', { caseSensitive: false })", lazy(&["B.ts", "a.ts"], "")),
        ("import.meta.glob('./m/b.ts', { caseSensitive: false })", lazy(&["B.ts"], "")),
        ("import.meta.glob(['./m/*.ts', '!./m/b.ts'], { caseSensitive: false })", lazy(&["a.ts"], "")),
        ("import.meta.glob('./m/*.TS')", "({})".to_string()),
        (
            "import.meta.glob('./m/*.ts', { query: { a: Math.PI, b: Number.MAX_SAFE_INTEGER, c: globalThis.undefined ?? 'n', d: 2 * 3 - 1 } })",
            lazy(&["B.ts", "a.ts"], "?a=3.141592653589793&b=9007199254740991&c=n&d=5"),
        ),
        (
            "import.meta.glob('./m/*.ts', { eager: 1 > 0 && 'a' < 'b', import: typeof x === 'undefined' ? 'a' : 'b' })",
            eager("a"),
        ),
        (
            "import.meta.glob('./m/*.ts', { eager: { e: true }.e, import: 'ab'[1] })",
            eager("b"),
        ),
    ];
    for (src, want) in expanded {
        assert_eq!(
            expand(&file, src, &d.0, &[]).as_deref(),
            Some(want.as_str()),
            "{src}"
        );
    }
    for (src, message) in [
        ("import.meta.glob('./m/*.ts', { eager: globalThis.x })", "Expected glob option \"eager\" to be of type boolean, but got undefined"),
        ("import.meta.glob('./m/*.ts', { eager: Boolean })", "Expected glob option \"eager\" to be of type boolean, but got function"),
        ("import.meta.glob('./m/*.ts', { eager: Number.MAX_SAFE_INTEGER })", "Expected glob option \"eager\" to be of type boolean, but got number"),
        ("import.meta.glob('./m/*.ts', { eager: 'x'.length })", "Expected glob option \"eager\" to be of type boolean, but got number"),
        ("import.meta.glob('./m/*.ts', { query: { a: JSON } })", "Expected glob option \"query.a\" to be of type string, number, or boolean, but got object"),
        ("import.meta.glob('./m/*.ts', { eager: undefined.x })", "Vite is unable to parse the glob options as the value is not static"),
        ("import.meta.glob('./m/*.ts', { eager: y.x })", "Vite is unable to parse the glob options as the value is not static"),
    ] {
        let message = serde_json::to_string(message).unwrap();
        assert_eq!(
            expand(&file, src, &d.0, &[]),
            Some(format!("throw new Error({message});{src}")),
            "{src}"
        );
    }
}
