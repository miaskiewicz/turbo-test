use super::*;
use std::path::Path;

const ROOT: &str = "/p";

fn err(file: &str, src: &str) -> Option<(&'static str, String)> {
    vite_parse_error(&Path::new(ROOT).join(file), src, Path::new(ROOT))
}

#[test]
fn a_module_that_parses_or_is_not_javascript_is_left_to_v8() {
    assert_eq!(err("a.js", "export const a = /x/u;\n"), None);
    assert_eq!(err("a.ts", "export const = ;\n"), None);
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
    let (_, m) = vite_parse_error(Path::new("/q/x.cjs"), src, Path::new(ROOT)).unwrap();
    assert_eq!(
        m,
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
