//! Unit tests for the oxc TS/JSX transform entry points.

use super::*;
use std::path::Path;

#[test]
fn needs_transform_by_extension() {
    for f in ["a.ts", "a.tsx", "a.jsx", "a.mts", "a.cts", "dir.x/a.d.ts"] {
        assert!(needs_transform(Path::new(f)), "{f}");
    }
    for f in ["a.js", "a.mjs", "a.cjs", "a.json", "Makefile", "a.ts.bak"] {
        assert!(!needs_transform(Path::new(f)), "{f}");
    }
}

#[test]
fn ts_types_are_stripped_and_exports_kept() {
    let src = "interface P { n: number }\ntype G = string;\n\
               function id<T>(x: T): T { return x; }\n\
               export const msg: G = id<string>(`hi ${({ n: 1 } as P).n}`);\n\
               export abstract class A { private x!: number; constructor(public y: number) {} }";
    let out = transform(Path::new("s.ts"), src).unwrap();
    assert!(!out.contains("interface"), "{out}");
    assert!(!out.contains(": G"), "{out}");
    assert!(!out.contains("<T>"), "{out}");
    assert!(
        out.contains("export const msg = id(`hi ${{ n: 1 }.n}`);"),
        "{out}"
    );
    assert!(out.contains("export class A"), "{out}");
    assert!(
        out.contains("this.y = y;"),
        "parameter property lowered: {out}"
    );
}

#[test]
fn mts_and_cts_are_typescript() {
    assert_eq!(
        transform(Path::new("m.mts"), "export const a: number = 1;").unwrap(),
        "export const a = 1;\n"
    );
    assert_eq!(
        transform(Path::new("c.cts"), "const a: number = 1;").unwrap(),
        "const a = 1;\n"
    );
}

#[test]
fn jsx_and_tsx_use_the_automatic_runtime() {
    let jsx = transform(
        Path::new("c.jsx"),
        "export const C = () => <div id=\"x\">hi</div>;",
    )
    .unwrap();
    assert!(jsx.contains("from \"react/jsx-runtime\""), "{jsx}");
    assert!(jsx.contains("jsx(\"div\", {"), "{jsx}");
    assert!(!jsx.contains("<div"), "{jsx}");

    let tsx = transform(
        Path::new("c.tsx"),
        "export const C = (p: { a: string }) => <>{p.a}</>;",
    )
    .unwrap();
    assert!(tsx.contains("Fragment"), "{tsx}");
    assert!(!tsx.contains(": {"), "{tsx}");
}

#[test]
fn errors_name_their_stage() {
    let e = transform(Path::new("noext"), "x").unwrap_err();
    assert!(e.starts_with("source type: "), "{e}");
    let e = transform(Path::new("a.ts"), "const = ;").unwrap_err();
    assert!(e.starts_with("parse errors: "), "{e}");
    let e = transform(Path::new("a.jsx"), "const a = <a:b />;").unwrap_err();
    assert!(e.starts_with("transform errors: "), "{e}");
    assert!(e.contains("Namespace tags are not supported"), "{e}");
}

const DECORATED: &str = "function dec(..._a: any[]): any {}\n\
    export class Svc {\n\
      @dec name?: string;\n\
      @dec other: number | null = null;\n\
      constructor(@dec dep: Svc) {}\n\
    }";

#[test]
fn transform_with_legacy_decorators_emits_metadata() {
    let out = transform_with(Path::new("svc.ts"), DECORATED, true).unwrap();
    assert!(out.contains("babelHelpers.decorate("), "{out}");
    assert!(
        out.contains("babelHelpers.decorateMetadata(\"design:type\", String)"),
        "optional field → String: {out}"
    );
    assert!(
        out.contains("babelHelpers.decorateMetadata(\"design:type\", Number)"),
        "T | null → T: {out}"
    );
    assert!(out.contains("babelHelpers.decorateParam(0, dec)"), "{out}");
    assert!(
        out.contains("\"design:paramtypes\", [typeof Svc === \"undefined\" ? Object : Svc]"),
        "{out}"
    );
    assert!(out.contains("export { Svc };"), "{out}");

    let default = transform_with(Path::new("svc.ts"), DECORATED, false).unwrap();
    assert!(!default.contains("babelHelpers"), "{default}");
    assert_eq!(transform(Path::new("svc.ts"), DECORATED).unwrap(), default);
}

#[test]
fn decorators_with_metadata_keep_esm_syntax() {
    let out = transform_decorators_with_metadata(Path::new("svc.ts"), DECORATED).unwrap();
    assert!(out.contains("let Svc = class Svc {"), "{out}");
    assert!(out.contains("export { Svc };"), "{out}");
    assert!(!out.contains("require("), "{out}");
    assert!(
        out.contains("babelHelpers.decorateMetadata(\"design:type\", String)"),
        "{out}"
    );
    assert_eq!(
        out,
        transform_with(Path::new("svc.ts"), DECORATED, true).unwrap()
    );
}

#[test]
fn decorators_with_metadata_error_stages() {
    let e = transform_decorators_with_metadata(Path::new("noext"), "x").unwrap_err();
    assert!(e.starts_with("source type: "), "{e}");
    let e = transform_decorators_with_metadata(Path::new("a.ts"), "class {").unwrap_err();
    assert!(e.starts_with("parse errors: "), "{e}");
    let e =
        transform_decorators_with_metadata(Path::new("a.tsx"), "const a = <a:b />;").unwrap_err();
    assert!(e.starts_with("transform errors: "), "{e}");
}

#[test]
fn maybe_transform_passes_plain_js_through() {
    let js = "export const a = 1 ;  // untouched\n".to_string();
    assert_eq!(maybe_transform(Path::new("a.mjs"), js.clone()).unwrap(), js);
    assert_eq!(
        maybe_transform(Path::new("a.ts"), "export const a: number = 1;".into()).unwrap(),
        "export const a = 1;\n"
    );
    assert!(maybe_transform(Path::new("a.ts"), "const = ;".into())
        .unwrap_err()
        .starts_with("parse errors: "));
}

/// Numeric, string, member-referencing, computed, cross-enum, exported, `const` and `declare`
/// enums. The member lines asserted below are tsc 5.9.3's output for the same source
/// (`tsc --target es2022 --module esnext`), the oracle for enum lowering.
const ENUMS: &str = "enum N { A, B, C = 10, D }\n\
    enum S { A = 'a', B = A, C = `${A}c` }\n\
    enum M { X = 1, Y = X * 2, Z = Y | 4, W = N.D }\n\
    enum C { K = 'k'.length, L = 5 }\n\
    const enum CE { P = 1, Q = 'q' }\n\
    export const v = [CE.P, CE.Q, N.D, S.B, M.Z, C.L];\n\
    declare enum D { Z }\n\
    export enum X { One = 1, Two = One + 1 }\n";

/// tsc's member assignments for `ENUMS`: string members get no reverse mapping, and members
/// that reference other members are folded to their constant value.
const TSC_ENUM_MEMBERS: &[&str] = &[
    "N[N[\"A\"] = 0] = \"A\";",
    "N[N[\"B\"] = 1] = \"B\";",
    "N[N[\"C\"] = 10] = \"C\";",
    "N[N[\"D\"] = 11] = \"D\";",
    "S[\"A\"] = \"a\";",
    "S[\"B\"] = \"a\";",
    "S[\"C\"] = \"ac\";",
    "M[M[\"X\"] = 1] = \"X\";",
    "M[M[\"Y\"] = 2] = \"Y\";",
    "M[M[\"Z\"] = 6] = \"Z\";",
    "M[M[\"W\"] = 11] = \"W\";",
    "C[C[\"K\"] = \"k\".length] = \"K\";",
    "C[C[\"L\"] = 5] = \"L\";",
    "X[X[\"One\"] = 1] = \"One\";",
    "X[X[\"Two\"] = 2] = \"Two\";",
];

/// Assert `out` lowers `ENUMS` with tsc's member values and no reverse mapping for strings.
fn assert_enums_like_tsc(out: &str) {
    for line in TSC_ENUM_MEMBERS {
        assert!(out.contains(line), "missing tsc line {line}:\n{out}");
    }
    assert!(
        !out.contains("S[S["),
        "string member got a reverse mapping:\n{out}"
    );
    assert!(!out.contains("var D"), "declare enum emitted:\n{out}");
}

#[test]
fn enums_lower_like_tsc() {
    // Runs in a debug build, so it would also trip oxc's `with_enum_eval` debug_assert.
    let out = transform(Path::new("e.ts"), ENUMS).unwrap();
    assert_enums_like_tsc(&out);
    // A `const enum` keeps a runtime object with tsc's member values and its accesses read
    // it (oxc's default `optimize_const_enums = false`, the `isolatedModules` behaviour);
    // tsc would inline `1 /* CE.P */`, which evaluates to the same value.
    assert!(out.contains("CE[CE[\"P\"] = 1] = \"P\";"), "{out}");
    assert!(out.contains("CE[\"Q\"] = \"q\";"), "{out}");
    assert!(
        out.contains("export const v = [\n\tCE.P,\n\tCE.Q,"),
        "{out}"
    );
}

#[test]
fn enums_lower_like_tsc_with_legacy_decorators() {
    assert_enums_like_tsc(&transform_with(Path::new("e.ts"), ENUMS, true).unwrap());
    assert_enums_like_tsc(&transform_decorators_with_metadata(Path::new("e.ts"), ENUMS).unwrap());
}

#[test]
fn module_syntax_follows_node_detection() {
    // Node 26 `node x.js` with no package.json "type": ESM for these ...
    for src in [
        "import x from 'y';",
        "export const a = 1;",
        "export * from './b';",
        "export default 1;",
        "console.log(import.meta.url);",
        "await Promise.resolve();",
        "const el = <i />;\nexport { el };",
    ] {
        assert!(has_module_syntax(src), "{src}");
    }
    // ... and CommonJS for these: words in comments / strings, require, dynamic import()
    for src in [
        "",
        "// import these where the editor renders\nexports.a = 1;",
        "/* export default */ module.exports = 1;",
        "const s = 'import x from y'; exports.s = s;",
        "const m = require('m');",
        "import('./lazy').then(() => {});",
        "var await = 1; console.log(await);",
        "this is not javascript",
    ] {
        assert!(!has_module_syntax(src), "{src}");
    }
}
