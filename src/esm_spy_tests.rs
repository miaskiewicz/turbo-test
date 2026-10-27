use super::*;

fn run(code: &str) -> Option<String> {
    spyable(Path::new("/p/m.js"), code)
}

#[test]
fn every_export_form_gets_a_setter() {
    let code = "import d, { a as b } from './x';\nimport * as all from './y';\n\
                export const c = 1, { e } = o;\nexport function f() {}\nexport class K {}\n\
                const g = 2;\nexport { g as h, b, d as i, all };\nexport let j = 3;\n\
                export { k as l } from './z';\nexport * from './s';\nexport * as t from './t';\n\
                export default () => 1;";
    assert_eq!(
        run(code).unwrap(),
        "import d, { a as b } from './x';\nimport * as all from './y';\n\
         export let   c = 1, { e } = o;\nexport function f() {}\nexport class K {}\n\
         let   g = 2;\nexport { g as h, b, d as i, all };\nexport let j = 3;\n\
         export { k as l } from './z';\nexport * from './s';\nexport * as t from './t';\n\
         let __tt_default = ({ default: () => 1 }).default;\n\
         import * as __tt_ns_0 from \"./x\";import * as __tt_ns_1 from \"./z\";import * as __tt_ns_2 from \"./s\";\
         import * as __tt_self from \"./m.js\";export { __tt_default as default };\
         globalThis.__ttNsSetter?.(__tt_self, (k, v) => { switch (k) { \
         case \"c\": c = v; return true; case \"e\": e = v; return true; case \"f\": f = v; return true; \
         case \"K\": K = v; return true; case \"h\": g = v; return true; \
         case \"b\": return globalThis.__ttNsSet(__tt_ns_0, \"a\", v); \
         case \"i\": return globalThis.__ttNsSet(__tt_ns_0, \"default\", v); case \"all\": all = v; return true; \
         case \"j\": j = v; return true; case \"l\": return globalThis.__ttNsSet(__tt_ns_1, \"k\", v); \
         case \"default\": __tt_default = v; return true; } \
         if (k !== \"default\" && k in __tt_ns_2) return globalThis.__ttNsSet(__tt_ns_2, k, v); return false; });\n"
    );
}

#[test]
fn anonymous_default_declarations_get_a_binding() {
    assert_eq!(
        run("export default async function* (a) {}\n").unwrap(),
        "export default async function*  __tt_default(a) {}\n\
         import * as __tt_self from \"./m.js\";Object.defineProperty(__tt_default, \"name\", { value: \"default\" });\
         globalThis.__ttNsSetter?.(__tt_self, (k, v) => { switch (k) { case \"default\": __tt_default = v; return true; } return false; });\n"
    );
    assert_eq!(
        run("export default class extends B {}").unwrap(),
        "export default class __tt_default extends B {}\n\
         import * as __tt_self from \"./m.js\";Object.defineProperty(__tt_default, \"name\", { value: \"default\", configurable: true });\
         globalThis.__ttNsSetter?.(__tt_self, (k, v) => { switch (k) { case \"default\": __tt_default = v; return true; } return false; });\n"
    );
    assert!(run("export default function named() {}")
        .unwrap()
        .contains("case \"default\": named = v; return true;"));
    assert!(run("export default class N {}")
        .unwrap()
        .contains("case \"default\": N = v; return true;"));
}

#[test]
fn modules_without_reachable_exports_are_left_alone() {
    for code in [
        "const a = 1;",
        "export {};",
        "export * as t from './t';",
        "export { a } from './j.json' with { type: 'json' };",
        "export using x = y;",
        "export const = ;",
        "import './x'; // export",
    ] {
        assert_eq!(run(code), None, "{code}");
    }
}
