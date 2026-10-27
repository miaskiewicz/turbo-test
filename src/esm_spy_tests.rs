use super::*;

fn run(code: &str) -> Option<String> {
    spyable(Path::new("/p/m.js"), code)
}

#[test]
fn importers_read_imports_through_the_overrides() {
    let code = "import d, { a as b } from './x';\nimport * as ns from './y';\nimport { c } from './x';\n\
                import { expect } from 'vitest';\nimport fs from 'node:fs';\nimport j from './j.json' with { type: 'json' };\n\
                const o = { b, k: d, ns };\nnew b(c, ns.q, typeof d, expect, fs, j);\n\
                function f(b) { return b; }\nimport('./z').then(f);\nexport { b, d as e };\n";
    assert_eq!(
        run(code).unwrap(),
        "import d, { a as b } from './x';\nimport * as ns from './y';\nimport { c } from './x';\n\
         import { expect } from 'vitest';\nimport fs from 'node:fs';\nimport j from './j.json' with { type: 'json' };\n\
         const o = { b: (globalThis.__ttR(__tt_ns_0, \"a\")), k: (globalThis.__ttR(__tt_ns_0, \"default\")), ns: (globalThis.__ttNs(ns)) };\n\
         new (globalThis.__ttR(__tt_ns_0, \"a\"))((globalThis.__ttR(__tt_ns_0, \"c\")), (globalThis.__ttNs(ns)).q, typeof (globalThis.__ttR(__tt_ns_0, \"default\")), expect, fs, j);\n\
         function f(b) { return b; }\n(import('./z')).then(globalThis.__ttNs).then(f);\nexport { b, d as e };\n\
         import * as __tt_ns_0 from \"./x\";import * as __tt_self from \"./m.js\";\
         globalThis.__ttNsRegister?.(__tt_self, [], {\"b\": [__tt_ns_0, \"a\"], \"e\": [__tt_ns_0, \"default\"]}, []);\n"
    );
}

#[test]
fn exporters_register_own_names_reexports_and_stars() {
    let code = "export const a = 1, { b } = o;\nexport function f() {}\nexport class K {}\nconst g = 2;\n\
                export { g as h };\nexport let j;\nexport { k as l } from './z';\nexport * from './s';\n\
                export * as t from './t';\nexport default 1;";
    assert_eq!(
        run(code).unwrap(),
        format!(
            "{code}\nimport * as __tt_ns_0 from \"./z\";import * as __tt_ns_1 from \"./s\";\
             import * as __tt_self from \"./m.js\";globalThis.__ttNsRegister?.(__tt_self, \
             [\"a\", \"b\", \"f\", \"K\", \"h\", \"j\", \"t\", \"default\"], {{\"l\": [__tt_ns_0, \"k\"]}}, [__tt_ns_1]);\n"
        )
    );
}

#[test]
fn modules_with_nothing_to_rewrite_are_left_alone() {
    for code in [
        "const a = 1;",
        "import './side-effect';",
        "import { it } from 'vitest'; it('x', () => {});",
        "import j from './j.json' with { type: 'json' }; j;",
        "export using x = y;",
        "export const = ;",
        "import { w } from './w'; w = 1;",
    ] {
        assert_eq!(run(code), None, "{code}");
    }
}
