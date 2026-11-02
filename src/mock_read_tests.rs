use super::guard_default_reads;

fn guard(src: &str) -> Option<String> {
    guard_default_reads(src, &|spec| spec == "./m.js")
}

#[test]
fn wraps_every_read_of_a_guarded_default_import() {
    let src = "import M from './m.js';\nimport N from './n.js';\nconst t = typeof M, b = M ? 1 : 0, e = M === N;\nf(M.x, M(), new M());\n";
    assert_eq!(
        guard(src).unwrap(),
        "import M from './m.js';\nimport N from './n.js';\nconst t = typeof (globalThis.__ttMockRead(M)), b = (globalThis.__ttMockRead(M)) ? 1 : 0, e = (globalThis.__ttMockRead(M)) === N;\nf((globalThis.__ttMockRead(M)).x, (globalThis.__ttMockRead(M))(), new (globalThis.__ttMockRead(M))());\n"
    );
}

#[test]
fn a_default_named_import_counts_and_named_imports_do_not() {
    let src = "import { default as D, n } from './m.js';\nuse(D, n);\n";
    assert_eq!(
        guard(src).unwrap(),
        "import { default as D, n } from './m.js';\nuse((globalThis.__ttMockRead(D)), n);\n"
    );
}

#[test]
fn shorthand_properties_expand_and_re_exports_keep_the_binding() {
    let src = "import M from './m.js';\nexport { M, M as N };\nexport const o = { M, k: 1 };\n";
    assert_eq!(
        guard(src).unwrap(),
        "import M from './m.js';\nexport { M, M as N };\nexport const o = { M: globalThis.__ttMockRead(M), k: 1 };\n"
    );
}

#[test]
fn shadowed_names_and_other_modules_are_left_alone() {
    let src = "import M from './m.js';\nfunction f(M) { return M; }\nf(1);\n";
    assert_eq!(guard(src), None);
    assert_eq!(guard("import M from './x.js';\nM;\n"), None);
    assert_eq!(guard("import * as ns from './m.js';\nns.default;\n"), None);
    // not a module that parses
    assert_eq!(guard("import M from './m.js';\nM +;\n"), None);
}
