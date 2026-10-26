use super::*;

#[test]
fn bindings_are_the_top_level_names_the_factories_mention() {
    let src = "import { vi } from 'vitest'\n\
               import type { T } from './t'\n\
               import { U, type V, w } from './u'\n\
               import d, * as ns from './d'\n\
               let a = 1, { b, c: [e] } = o\n\
               var f\n\
               const g = 2\n\
               declare const h: number\n\
               function fn() {}\n\
               class K {}\n\
               enum E { X }\n\
               export const x = 1\n\
               export function xf() {}\n\
               export default function df() {}\n\
               export { a as aa }\n\
               interface I {}\n\
               const unused = 3\n\
               if (a) {}\n\
               const u: U = w\n\
               vi.mock('./m', () => ({ m: () => d + ns.y }))\n";
    let factories = "vi T U V w d ns a b e f g h fn K E x xf df I u";
    assert_eq!(
        bindings(Path::new("a.ts"), src, factories),
        [
            // `U` is only a type (no value reference); `vi` comes from the test API; `d` / `ns`
            // are read by the factory
            ("w".to_string(), false),
            ("d".to_string(), false),
            ("ns".to_string(), false),
            ("a".to_string(), true),
            ("b".to_string(), true),
            ("e".to_string(), true),
            ("f".to_string(), true),
            ("g".to_string(), false),
            ("fn".to_string(), true),
            ("K".to_string(), false),
            ("E".to_string(), false),
            ("x".to_string(), false),
            ("xf".to_string(), true),
            ("df".to_string(), true),
            ("u".to_string(), false),
        ]
    );
    // a default-exported class; anonymous defaults and expressions bind nothing
    let src = "export default class DK {}\n";
    assert_eq!(
        bindings(Path::new("a.js"), src, "DK"),
        [("DK".to_string(), false)]
    );
    for src in [
        "export default function () {}",
        "export default class {}",
        "export default 1",
    ] {
        assert!(bindings(Path::new("a.js"), src, "x").is_empty(), "{src}");
    }
}

#[test]
fn mentions_whole_identifiers_only() {
    assert!(mentions("() => a + b", "a") && mentions("x$y", "x$y"));
    assert!(!mentions("ab $a a_ a1", "a"));
    assert!(mentions("ab a", "a"));
}

#[test]
fn binder_and_scope_text() {
    let b = [("a".to_string(), true), ("K".to_string(), false)];
    assert_eq!(
        binder(&b),
        "globalThis.__ttScopeBind({ a: () => a, K: () => K }, { a: (v) => { a = v; } });"
    );
    assert_eq!(
        with_scope("f()", &b),
        "globalThis.__ttScopeNames([\"a\", \"K\"]);\nwith (globalThis.__ttScope) {\nf()\n}"
    );
}

#[test]
fn mock_calls_are_read_from_the_ast() {
    let calls = |src: &str| mock_calls(src, &["vi.mock", "jest.mock"]);
    let pairs = |v: &[(&str, &str)]| {
        Some(
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>(),
        )
    };
    // a ')' inside a string or template belongs to the factory (the text scan cut it there)
    assert_eq!(
        calls("vi.mock('./a', () => ({ s: 'a)', t: `:)` }));"),
        pairs(&[("./a", "() => ({ s: 'a)', t: `:)` })")])
    );
    // calls in comments and strings are not calls; a template specifier without substitutions
    // counts, a computed one and a non-literal do not; a third argument is not the factory
    assert_eq!(
        calls(
            "// vi.mock('./c', () => 1)\nconst s = \"vi.mock('./s', f)\";\n\
             vi.mock(`./t`, () => 2, { spy: true });\nvi.mock(`./${x}`, () => 3);\nvi.mock(name, () => 4);\n\
             jest.mock('./j');"
        ),
        pairs(&[("./t", "() => 2"), ("./j", "")])
    );
    // TypeScript and JSX factories; a call inside a matched factory is part of it, not another
    assert_eq!(
        calls("vi.mock('./d', () => ({ g: (n: string) => <i>{n}</i>, h: vi.mock('./inner') }));"),
        pairs(&[(
            "./d",
            "() => ({ g: (n: string) => <i>{n}</i>, h: vi.mock('./inner') })"
        )])
    );
    // only the named callees, only as `object.property` of an identifier
    assert_eq!(
        calls("a.vi.mock('./x', f); vi['mock']('./y', f); mock('./z', f); vi.doMock('./w', f);"),
        pairs(&[])
    );
    assert_eq!(
        mock_calls("vi.doMock('./w', f);", &["vi.doMock"]),
        pairs(&[("./w", "f")])
    );
    // a `<T>x` cast is not TSX: the TypeScript parse reads it
    assert_eq!(
        calls("const n = <number>1;\nvi.mock('./n', () => ({ n }));"),
        pairs(&[("./n", "() => ({ n })")])
    );
    // source that parses as neither: None (the caller scans the text)
    assert_eq!(calls("vi.mock('./u', () => (1"), None);
}
