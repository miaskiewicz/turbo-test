use super::*;

const C: Colors = Colors { on: true };

/// `tests/data/highlight-corpus.json`: `[file, source, tokens | null, highlighted]`, the tokens
/// and the highlighting vitest 4.1.12's own js-tokens / tinyhighlight / `highlightCode` give
/// (colours forced on) for edge-case snippets and some of the repo's test files.
/// A corpus case: file, source, its tokens (type, text) when recorded, the highlighting.
type Case = (String, String, Option<Vec<(String, String)>>, String);

fn corpus() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/highlight-corpus.json"
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let s = |i: usize| c[i].as_str().unwrap().to_string();
            let tokens = c[2].as_array().map(|t| {
                t.iter()
                    .map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into()))
                    .collect()
            });
            (s(0), s(1), tokens, s(3))
        })
        .collect()
}

#[test]
fn tokens_are_js_tokens() {
    let mut checked = 0;
    for (file, src, tokens, _) in corpus() {
        let Some(expected) = tokens else { continue };
        let got: Vec<(String, String)> = super::tokens(&src, file.ends_with('x'))
            .iter()
            .map(|t| (format!("{:?}", t.kind), t.value.to_string()))
            .collect();
        assert_eq!(got, expected, "{file}: {src:?}");
        checked += 1;
    }
    assert!(checked >= 20, "{checked}");
}

#[test]
fn highlighting_is_vitests() {
    for (file, src, _, expected) in corpus() {
        assert_eq!(
            highlight_code(&C, Path::new(&file), &src),
            expected,
            "{file}: {src:?}"
        );
    }
}

#[test]
fn only_js_and_ts_are_highlighted() {
    for ext in [
        "js", "mjs", "cjs", "jsx", "mjsx", "ts", "mts", "cts", "tsx", "ctsx",
    ] {
        let f = format!("a.{ext}");
        assert_ne!(highlight_code(&C, Path::new(&f), "1;"), "1;", "{f}");
    }
    for f in ["a.css", "a.json", "a", "a.mmjs", "a.jsxx", "a.vue"] {
        assert_eq!(highlight_code(&C, Path::new(f), "1;"), "1;", "{f}");
    }
    // colours off: nothing changes
    assert_eq!(
        highlight_code(&Colors { on: false }, Path::new("a.ts"), "it(1);"),
        "it(1);"
    );
}

#[test]
fn scanners_follow_the_regular_expressions() {
    // a class that never closes is a plain `[`
    assert_eq!(regex_literal("/[a\n", 0), 3);
    assert_eq!(regex_literal("/a\\", 0), 3);
    assert_eq!(regex_literal("//", 0), 0);
    assert_eq!(numeric_literal("0_1", 0), Some(1));
    assert_eq!(numeric_literal("1_", 0), Some(1));
    assert_eq!(numeric_literal("0x", 0), Some(1));
    assert_eq!(numeric_literal("x", 0), None);
    assert_eq!(numeric_literal("1e+", 0), Some(1));
    assert_eq!(identifier("#", 0), None);
    assert_eq!(identifier("\\u{}", 0), None);
    assert_eq!(identifier("\\u{41}b", 0), Some((7, false)));
    assert_eq!(template("`a\\", 0), Some((2, "")));
    assert_eq!(string_literal("'a\\", 0), Some(2));
    assert_eq!(punctuator("@", 0), None);
}
