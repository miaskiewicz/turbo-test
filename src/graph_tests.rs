//! Unit tests for static specifier extraction, the transitive import closure and the
//! affected-test selection used by watch mode.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A fresh scratch project directory (outside the repo) with `files` written into it.
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("turbo-test-graph-ut")
        .join(format!("{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // stale dir from a recycled pid
    std::fs::create_dir_all(&dir).unwrap();
    for (rel, src) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    dir.canonicalize().unwrap()
}

#[test]
fn extract_specifiers_covers_every_form_in_keyword_order() {
    let src = "import a from './a';\nimport './b';\nconst c = require( \"./c\" );\n\
               const d = import(`./d`);\nexport * from \"./e\";\n\
               xfrom './no1'; ximport('./no2'); myrequire('./no3');\n\
               const importer = 1; from; import\n";
    assert_eq!(
        extract_specifiers(src),
        vec!["./a", "./e", "./b", "./d", "./c"],
        "all `from` hits, then `import`, then `require`"
    );
}

#[test]
fn extract_specifiers_edge_cases() {
    assert!(extract_specifiers("").is_empty());
    assert!(
        extract_specifiers("import 'unterminated").is_empty(),
        "no closing quote"
    );
    assert!(
        extract_specifiers("require").is_empty(),
        "keyword at end of input"
    );
    assert_eq!(
        extract_specifiers("_from'x'"),
        vec!["x"],
        "only alphanumerics break the word boundary"
    );
    assert_eq!(extract_specifiers("require(\n  'a'\n)"), vec!["a"]);
    assert_eq!(
        extract_specifiers("import ''"),
        vec![""],
        "empty specifier is still extracted"
    );
}

#[test]
fn is_node_modules_checks_components() {
    assert!(is_node_modules(Path::new("/p/node_modules/x/index.js")));
    assert!(!is_node_modules(Path::new("/p/my_node_modules/x.js")));
    assert!(!is_node_modules(Path::new("/p/src/x.js")));
}

fn graph_project() -> PathBuf {
    project(
        "graph",
        &[
            ("src/a.js", "import { b } from './b.js';\nexport const a = b;\n"),
            ("src/b.js", "import { a } from './a.js';\nimport p from 'pkg';\nimport m from './missing';\nexport const b = 1;\n"),
            ("src/c.js", "export const c = 3;\n"),
            ("node_modules/pkg/package.json", "{\"name\":\"pkg\",\"main\":\"index.js\"}"),
            ("node_modules/pkg/index.js", "module.exports = require('./inner.js');\n"),
            ("node_modules/pkg/inner.js", "module.exports = 1;\n"),
            ("t1.test.js", "import { a } from './src/a.js';\n"),
            ("t2.test.js", "import { c } from './src/c';\n"),
            ("t3.test.js", "test('standalone', () => {});\n"),
        ],
    )
}

#[test]
fn transitive_imports_is_cycle_safe_and_stops_at_node_modules() {
    let dir = graph_project();
    let mut memo = HashMap::new();
    let deps = transitive_imports(&dir.join("t1.test.js"), &mut memo);
    let mut names: Vec<String> = deps
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["node_modules/pkg/index.js", "src/a.js", "src/b.js"]
    );
    // memoized: every visited file has an entry and a second call returns the same set
    assert!(memo.contains_key(&dir.join("src/b.js")));
    assert_eq!(transitive_imports(&dir.join("t1.test.js"), &mut memo), deps);
    // in the a <-> b cycle, b's closure contains a (and, through it, b itself)
    let b = transitive_imports(&dir.join("src/b.js"), &mut memo);
    assert!(b.contains(&dir.join("src/a.js")), "{b:?}");
}

#[test]
fn transitive_imports_of_an_unreadable_file_is_empty() {
    let dir = graph_project();
    let mut memo = HashMap::new();
    assert!(transitive_imports(&dir.join("nope.js"), &mut memo).is_empty());
    assert!(
        transitive_imports(Path::new(""), &mut memo).is_empty(),
        "no parent dir"
    );
}

#[test]
fn affected_tests_selects_importers_and_changed_tests() {
    let dir = graph_project();
    let tests: Vec<PathBuf> = ["t1.test.js", "t2.test.js", "t3.test.js"]
        .iter()
        .map(|t| dir.join(t))
        .collect();

    // a dependency change (given via a non-canonical path) selects only its importers
    let changed = vec![dir.join("src/../src/b.js")];
    assert_eq!(affected_tests(&changed, &tests), vec![tests[0].clone()]);

    // a changed test file is always selected; unrelated non-existent paths select nothing
    let changed = vec![dir.join("t3.test.js"), dir.join("gone.js")];
    assert_eq!(affected_tests(&changed, &tests), vec![tests[2].clone()]);

    // a node_modules entry change reaches every test that imports the package
    let changed = vec![dir.join("node_modules/pkg/index.js")];
    assert_eq!(affected_tests(&changed, &tests), vec![tests[0].clone()]);

    // a change below the node_modules leaf is NOT traced (a dep change triggers a full run)
    let changed = vec![dir.join("node_modules/pkg/inner.js")];
    assert!(affected_tests(&changed, &tests).is_empty());

    assert!(affected_tests(&[], &tests).is_empty());
}
