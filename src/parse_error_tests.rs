use super::*;
use std::path::{Path, PathBuf};

const ROOT: &str = "/p";

fn none(_: &str) -> Option<PathBuf> {
    None
}

fn err(file: &str, src: &str) -> Option<(&'static str, String)> {
    vite_error(&Path::new(ROOT).join(file), src, Path::new(ROOT), &none)
        .map(|e| (e.name, e.message))
}

#[test]
fn a_module_that_parses_or_is_not_javascript_is_left_to_v8() {
    assert_eq!(err("a.js", "export const a = /x/u;\n"), None);
    assert_eq!(err("a.ts", "export const a: number = /x/u;\n"), None);
    assert_eq!(err("a", "export const = ;\n"), None);
}

#[test]
fn a_parse_error_is_rolldowns_parse_failure() {
    // vitest 4.1.12 for syn.mjs = `export const = ;`
    assert_eq!(
        err("syn.mjs", "export const = ;\n"),
        Some((
            "RolldownError",
            "Parse failure: Parse failed with 1 error:\nUnexpected token\n1: export const = ;\n                ^\nAt file: /syn.mjs:1:13".to_string()
        ))
    );
    // the frame: two lines before, two after, numbers padded; outside the root: /@fs
    let src = "a;\nb;\nc;\nconst = 1;\nd;\ne;\nf;\n";
    let e = vite_parse_error(Path::new("/q/x.cjs"), src, Path::new(ROOT), &none).unwrap();
    assert_eq!(
        e.message,
        "Parse failure: Parse failed with 1 error:\nUnexpected token\n2: b;\n3: c;\n4: const = 1;\n         ^\n5: d;\n6: e;\nAt file: /@fs/q/x.cjs:4:6"
    );
}

#[test]
fn code_frames_number_cut_and_trim() {
    let long = format!("{}x", "a".repeat(130));
    assert_eq!(
        code_frame(&format!("{long}\n"), 1, 2),
        format!("1: {}...\n     ^", "a".repeat(117))
    );
    assert_eq!(code_frame("a\n\n\n", 1, 0), "1: a\n   ^");
    assert_eq!(code_frame("a", 3, 0), "");
    let ten = (1..=11)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code_frame(&ten, 10, 1),
        " 8: 8\n 9: 9\n10: 10\n     ^\n11: 11"
    );
    assert_eq!(code_frame("\tx\ty", 1, 1), "1:   x\ty\n     ^");
    assert_eq!(locate("ab\ncd", 4), (2, 1));
}

#[test]
fn what_the_lexer_rejects_is_vites_import_analysis_error() {
    let ia = Some(("Error", IMPORT_ANALYSIS.to_string()));
    for src in [
        "test('t', () => {}); )\n",
        "}\nconst = 1;\n",
        "const a = 'x\nconst = 1;\n",
        "const a = `x${ 1 }\n",
        "const a = (1 + ;\n",
        "const r = = /x\n",
    ] {
        assert_eq!(err("e.js", src), ia, "{src:?}");
    }
}

#[test]
fn the_lexer_skips_strings_comments_templates_and_regexes() {
    for src in [
        "const a = ')'; const b = \")\"; const = 1;",
        "// )\n/* } */ const = 1;",
        "const t = `)${ ({}) }}`; const = 1;",
        "const r = /\\)[/)]/; const = 1;",
        "let x = 4 / 2 / 1; const = 1;",
        "f(a) / 2; const = 1;",
        "function g() { return /)/; } const = 1;",
        "const s = '\\''; const = 1;",
        "/* unterminated",
    ] {
        assert!(!lexer_rejects(src), "{src:?}");
        if src.contains("const = 1") {
            assert_eq!(err("e.js", src).unwrap().0, "RolldownError", "{src:?}");
        }
    }
    assert!(lexer_rejects("`x"));
    assert!(lexer_rejects("`${"));
    assert!(lexer_rejects("'x"));
    assert!(lexer_rejects("x = /a"));
    assert!(lexer_rejects("("));
}

#[test]
fn messages_are_vitests() {
    // vitest 4.1.12's json messages for these modules
    for (src, want) in [
        ("a b c d e f g h\n", "Parse failure: Parse failed with 1 error:\nExpected a semicolon or an implicit semicolon after a statement, but found none\n1: a b c d e f g h\n    ^\nAt file: /m.mjs:1:1"),
        ("\t\tconst = 1;\n", "Parse failure: Parse failed with 1 error:\nUnexpected token\n1:     const = 1;\n             ^\nAt file: /m.mjs:1:8"),
        ("let x = 4 / 2 / 1; const = 1;\n", "Parse failure: Parse failed with 1 error:\nUnexpected token\n1: let x = 4 / 2 / 1; const = 1;\n                            ^\nAt file: /m.mjs:1:25"),
        ("const t = `)${ ({}) }}`; const = 1;\n", "Parse failure: Parse failed with 1 error:\nUnexpected token\n1: const t = `)${ ({}) }}`; const = 1;\n                                  ^\nAt file: /m.mjs:1:31"),
        ("const a = (1 + ;\n", IMPORT_ANALYSIS),
    ] {
        assert_eq!(err("m.mjs", src).map(|e| e.1).as_deref(), Some(want), "{src:?}");
    }
}

#[test]
fn static_imports_are_told_from_requires() {
    let src = "import a from './a';\nimport './b';\nexport * from './c';\nexport { d } from './d';\nexport const e = 1;\nconst f = require('./f');\n";
    let path = Path::new("/p/m.ts");
    for spec in ["./a", "./b", "./c", "./d"] {
        assert!(imports_statically(path, src, spec), "{spec}");
    }
    for spec in ["./e", "./f", "a"] {
        assert!(!imports_statically(path, src, spec), "{spec}");
    }
}

#[test]
fn the_lexer_fails_where_es_module_lexer_does() {
    // es-module-lexer 1.x's `e.idx` for each source
    for (src, at) in [
        ("test('t', () => {}));\n", 19),
        ("a}\n", 1),
        ("const s = 'abc;\nx\n", 15),
        ("const s = 'abc", 14),
        ("f(`abc", 6),
        ("const r = /abc\n;", 14),
        ("f(1, {\n", 0),
        ("x = `${a`\n", 10),
        ("(()\n", 0),
        ("x = `${a}`; }", 12),
    ] {
        assert_eq!(lex(src).error, Some(at), "{src:?}");
    }
    assert_eq!(lex("a = \"x\\\ny\";\n").error, None);
}

#[test]
fn import_analysis_errors_carry_vites_position_and_frame() {
    let e = vite_error(
        Path::new("/p/ia.test.js"),
        "import { test } from 'vitest';\ntest('t', () => {}));\n",
        Path::new(ROOT),
        &none,
    )
    .unwrap();
    // vitest 4.1.12's Failed Suites entry for it
    assert_eq!(
        e,
        ViteError {
            name: "Error",
            message: IMPORT_ANALYSIS.to_string(),
            plugin: Some("vite:import-analysis"),
            id: "/p/ia.test.js".into(),
            loc: Some((2, 19)),
            frame: Some("1  |  import { test } from 'vitest';\n2  |  test('t', () => {}));\n   |                     ^\n3  |  ".into()),
        }
    );
    // a source that looks binary: no position, no frame
    let e = vite_error(Path::new("/p/b.js"), "\u{fffd})", Path::new(ROOT), &none).unwrap();
    assert_eq!((e.loc, e.frame), (None, None));
}

#[test]
fn parse_failures_frame_the_module_with_its_imports_rewritten() {
    let src = include_str!("../tests/data/transform-errors/pf.test.js");
    let resolve = |spec: &str| match spec {
        "./m.mjs" => Some(PathBuf::from("/p/m.mjs")),
        "./sub/z.mjs" => Some(PathBuf::from("/p/sub/z.mjs")),
        _ => None,
    };
    let e = vite_error(Path::new("/p/pf.test.js"), src, Path::new(ROOT), &resolve).unwrap();
    let want: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../tests/data/transform-errors/messages-vitest-4.1.12.json"
    ))
    .unwrap();
    assert_eq!(e.name, "RolldownError");
    assert_eq!(e.message, want["pf.test.js"]);
    assert_eq!(
        (e.plugin, e.id.as_str(), e.loc),
        (None, "/pf.test.js", Some((5, 6)))
    );
    assert_eq!(
        e.frame.as_deref(),
        Some("3  |  //\n4  |  import { y } from \"/m.mjs\"; import fs from 'node:fs';\n5  |  const = 1; import { z } from \"/sub/z.mjs\";\n   |        ^\n6  |  export { y as yy } from \"/m.mjs\";\n7  |  const d = import(\"/m.mjs\");")
    );
}

#[test]
fn import_specifiers_are_rewritten_to_vites_urls() {
    let src = "import a from './a'; import './b'; export * from 'c'; const d = import ( /* x */ './d');\nimport fs from 'node:fs'; f('./x'); from = './y'; import(`./t`); import { e } from './e\\'';\n";
    let lexed = lex(src);
    assert_eq!(lexed.error, None);
    let specs: Vec<&str> = lexed.imports.iter().map(|r| &src[r.clone()]).collect();
    assert_eq!(
        specs,
        ["'./a'", "'./b'", "'c'", "'./d'", "'node:fs'", "'./e\\''"]
    );
    let resolve = |spec: &str| match spec {
        "./a" => Some(PathBuf::from("/p/a.js")),
        "c" => Some(PathBuf::from("/q/c/index.js")),
        "./d" => Some(PathBuf::from("/p/sub/d.mjs")),
        "node:fs" | "./e\\'" => Some(PathBuf::from("/p/never.js")),
        _ => None,
    };
    assert_eq!(
        rewrite_imports(src, &lexed.imports, Path::new(ROOT), &resolve),
        "import a from \"/a.js\"; import './b'; export * from \"/@fs/q/c/index.js\"; const d = import ( /* x */ \"/sub/d.mjs\");\nimport fs from 'node:fs'; f('./x'); from = './y'; import(`./t`); import { e } from './e\\'';\n"
    );
}

#[test]
fn vites_code_frame_cuts_long_lines_around_the_position() {
    let long = "a".repeat(200);
    let src = format!("x\n{long}\ny\n{long}");
    // generateCodeFrame: the error line from 60 characters before the position, `...` before it
    assert_eq!(
        vite_code_frame(&src, 2 + 150),
        format!(
            "1  |  x\n2  |  ...{}\n   |  {}^\n3  |  y\n4  |  {}...",
            "a".repeat(110),
            " ".repeat(63),
            "a".repeat(117)
        )
    );
    // near its start: no leading cut
    assert_eq!(
        vite_code_frame(&src, 2 + 10),
        format!(
            "1  |  x\n2  |  {}...\n   |  {}^\n3  |  y\n4  |  {}...",
            "a".repeat(117),
            " ".repeat(10),
            "a".repeat(117)
        )
    );
    // the number column widens past two digits
    let lines = (1..=12)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\r\n");
    assert_eq!(
        vite_code_frame(&lines, lines.find("10").unwrap() + 1),
        "8  |  8\n9  |  9\n10 |  10\n   |   ^\n11 |  11\n12 |  12"
    );
}

#[test]
fn ts_and_jsx_transform_errors_are_vites_oxc_report() {
    let want: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../tests/data/transform-errors/messages-vitest-4.1.12.json"
    ))
    .unwrap();
    for (file, test) in [
        ("entrysyn.test.ts", "entrysyn.test.ts"),
        ("dep.ts", "tsdep.test.ts"),
        ("two.test.ts", "two.test.ts"),
        ("help.test.ts", "help.test.ts"),
        ("jsx.test.tsx", "jsx.test.tsx"),
    ] {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/transform-errors")
                .join(file),
        )
        .unwrap();
        let path = Path::new(ROOT).join(file);
        let e = vite_error(&path, &src, Path::new(ROOT), &none).unwrap();
        assert_eq!(
            e,
            ViteError {
                name: "Error",
                message: want[test].clone(),
                plugin: Some("vite:oxc"),
                id: path.display().to_string(),
                loc: None,
                frame: None,
            },
            "{file}"
        );
    }
    // named relative to the working directory; at most five errors, then `...`
    let six =
        "let a; let a; let b; let b; let c; let c; let d; let d; let e; let e; let f; let f;\n";
    let e = vite_error(Path::new("/p/sub/x.mts"), six, Path::new("/p/q"), &none).unwrap();
    assert!(
        e.message.starts_with("Transform failed with 6 errors:\n\n"),
        "{}",
        e.message
    );
    assert!(e.message.ends_with("╯\x1b[0m\n\n..."), "{:?}", e.message);
    assert_eq!(e.message.matches("[PARSE_ERROR]").count(), 5);
    assert!(e.message.contains(" ../sub/x.mts:1:5 "), "{}", e.message);
    // a module oxc takes (V8 alone may reject it) and other kinds are left alone
    assert_eq!(err("ok.tsx", "export const a = <div/>;\n"), None);
    assert_eq!(err("a.json", "{"), None);
}

#[test]
fn a_label_without_context_is_named_in_the_title() {
    // rolldown's is_context_too_long
    let flat = "a".repeat(700);
    assert!(context_too_long(&flat, 350, 351));
    assert!(context_too_long(&flat, 650, 800));
    assert!(!context_too_long(&"a".repeat(599), 300, 301));
    let lined = format!(
        "{}\n{}\n{}",
        "a".repeat(300),
        "b".repeat(100),
        "c".repeat(300)
    );
    assert!(!context_too_long(&lined, 350, 351));
    // both sides must be short or broken: near the end alone is not enough
    let near_end = format!("{}{}", "a".repeat(650), "b".repeat(10));
    assert!(context_too_long(&near_end, 655, 656));
    let near_start = format!("ab\n{}", "c".repeat(700));
    assert!(!context_too_long(&near_start, 0, 1));
    let src = format!("const a = 1 ?? 2 || 3; {}\n", "x".repeat(700));
    let shown = rolldown_diagnostic("m.ts", &src, "T", &[(10, 21, "L".into())], None);
    assert!(shown.contains("T\n - L in m.ts at 10..21"), "{shown}");
    assert!(!shown.contains("m.ts:1:11"), "{shown}");
    assert_eq!(relative(Path::new("/p/a/b.ts"), Path::new("/p")), "a/b.ts");
    assert_eq!(
        relative(Path::new("/p/a/b.ts"), Path::new("/p/c/d")),
        "../../a/b.ts"
    );
}
