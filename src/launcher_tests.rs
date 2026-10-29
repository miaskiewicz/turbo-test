//! Unit tests for `launcher.rs`: discovery, config scanning (include/exclude, environment,
//! coverage block, testTimeout, aliases), comment/regex stripping, `--changed` via a real git
//! repo, and `plan()` end-to-end (flag consumption, env vars, messages and exit decisions).

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique, empty, canonical scratch directory OUTSIDE the repo (removed on drop).
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Tmp {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join("turbo-test-unit");
        std::fs::create_dir_all(&base).unwrap();
        let dir = base
            .canonicalize()
            .unwrap()
            .join(format!("launcher-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
    fn file(&self, rel: &str, contents: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, contents).unwrap();
        p
    }
    fn p(&self, rel: &str) -> String {
        self.0.join(rel).display().to_string()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn run_plan(t: &Tmp, a: &[&str]) -> Plan {
    plan(args(a), &t.0)
}

fn argv(p: &Plan) -> Vec<String> {
    p.outcome
        .clone()
        .unwrap_or_else(|c| panic!("expected argv, got exit {c}: {:?}", p.messages))
}

fn env_of<'a>(p: &'a Plan, k: &str) -> Option<&'a str> {
    p.env
        .iter()
        .rev()
        .find(|(ek, _)| *ek == k)
        .map(|(_, v)| v.as_str())
}

const T: &str = "test('x', () => {})\n";

// ---- discovery ----------------------------------------------------------------------------

#[test]
fn test_file_names() {
    for n in [
        "a.test.ts",
        "a.spec.tsx",
        "b.test.js",
        "c.spec.jsx",
        "d.test.mts",
        "e.test.cts",
        "x.y.test.ts",
        // turbo-test#57: vitest's default include, `*.{test,spec}.?(c|m)[jt]s?(x)`
        "f.test.mjs",
        "g.test.cjs",
        "h.spec.mtsx",
        "i.test.cjsx",
    ] {
        assert!(is_test_file(n), "{n}");
    }
    for n in [
        "a.ts",
        "test.ts",
        "a.test.md",
        "a.tests.ts",
        "a.test",
        "noext",
        "a_test.ts",
        ".test.ts",
        "a.test.jsxx",
        "a.test.xjs",
    ] {
        assert_eq!(is_test_file(n), n == ".test.ts", "{n}");
    }
}

#[test]
fn walk_skips_only_node_modules_and_git() {
    // turbo-test#57: vitest's default exclude is `**/node_modules/**` and `**/.git/**`; hidden
    // entries and build output directories are discovered (vitest 4.1.12)
    let t = Tmp::new("walk");
    t.file("a.test.ts", T);
    t.file("src/deep/b.spec.js", T);
    t.file("src/helper.ts", "");
    t.file(".hidden/c.test.ts", T);
    t.file("src/.d.test.ts", T);
    for d in ["dist", "build", "coverage", ".next", ".turbo", "target"] {
        t.file(&format!("{d}/e.test.ts"), T);
    }
    for d in ["node_modules", ".git"] {
        t.file(&format!("{d}/e.test.ts"), T);
        t.file(&format!("src/{d}/f.test.ts"), T);
    }
    // a directory whose name looks like a test file is descended, not collected
    t.file("g.test.ts/inner.test.ts", T);
    // pruned by vitest's default exclude
    let def: Vec<String> = DEFAULT_EXCLUDE_GLOBS
        .iter()
        .map(|g| g.to_string())
        .collect();
    let root = t.0.clone();
    let prune =
        move |d: &Path| excludes_dir(&def, &d.strip_prefix(&root).unwrap().to_string_lossy());
    let mut out = Vec::new();
    walk(&t.0, &mut out, &is_test_file, &prune);
    out.sort();
    let want: Vec<PathBuf> = [
        ".hidden/c.test.ts",
        ".next/e.test.ts",
        ".turbo/e.test.ts",
        "a.test.ts",
        "build/e.test.ts",
        "coverage/e.test.ts",
        "dist/e.test.ts",
        "g.test.ts/inner.test.ts",
        "src/.d.test.ts",
        "src/deep/b.spec.js",
        "target/e.test.ts",
    ]
    .iter()
    .map(|f| t.0.join(f))
    .collect();
    assert_eq!(out, want);
    assert_eq!(DEFAULT_EXCLUDE_GLOBS, ["**/node_modules/**", "**/.git/**"]);
    // nothing pruned: those directories are searched too
    let mut all = Vec::new();
    walk(&t.0, &mut all, &is_test_file, &|_| false);
    assert_eq!(all.len(), want.len() + 4);
    // unreadable dir → nothing, no panic
    let mut none = Vec::new();
    walk(&t.0.join("missing"), &mut none, &is_test_file, &|_| false);
    assert!(none.is_empty());
}

#[cfg(unix)]
#[test]
fn walk_ignores_symlinks_and_specials() {
    let t = Tmp::new("walk-sym");
    let real = t.file("real/a.test.ts", T);
    std::os::unix::fs::symlink(&real, t.0.join("link.test.ts")).unwrap();
    std::os::unix::fs::symlink(t.0.join("real"), t.0.join("linkdir")).unwrap();
    let mut out = Vec::new();
    walk(&t.0, &mut out, &is_test_file, &|_| false);
    assert_eq!(out, vec![real]);
}

#[test]
fn excludes_dir_needs_a_glob_covering_the_whole_directory() {
    // turbo-test#63 item 3: discovery prunes a directory only when an exclude `<prefix>/**`
    // covers everything under it
    let ex = |g: &[&str]| -> Vec<String> { g.iter().map(|s| s.to_string()).collect() };
    let def = ex(&DEFAULT_EXCLUDE_GLOBS);
    assert!(excludes_dir(&def, "node_modules") && excludes_dir(&def, "a/b/.git"));
    assert!(!excludes_dir(&def, "src") && !excludes_dir(&def, "node_modules_x"));
    assert!(excludes_dir(&ex(&["e2e/**"]), "e2e") && !excludes_dir(&ex(&["e2e/**"]), "a/e2e"));
    // a file glob never prunes
    assert!(!excludes_dir(&ex(&["**/*.e2e.ts"]), "x"));
    assert!(!excludes_dir(&[], "node_modules"));
}

#[test]
fn discover_without_config_sorts_everything() {
    let t = Tmp::new("disc");
    t.file("z.test.ts", T);
    t.file("a/b.test.ts", T);
    assert_eq!(
        discover(&t.0, None),
        vec![t.0.join("a/b.test.ts"), t.0.join("z.test.ts")]
    );
}

#[test]
fn discover_honours_test_include_exclude() {
    let t = Tmp::new("disc-cfg");
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['src/**/*.test.ts', \"lib/*.spec.ts\"], exclude: ['src/skip/**'] } }",
    );
    t.file("src/a.test.ts", T);
    t.file("src/skip/b.test.ts", T);
    t.file("lib/c.spec.ts", T);
    t.file("other/d.test.ts", T);
    assert_eq!(
        discover(&t.0, None),
        vec![t.0.join("lib/c.spec.ts"), t.0.join("src/a.test.ts")]
    );
}

/// turbo-test#57: a config include is matched against every file, not only default test names.
#[test]
fn discover_matches_a_config_include_against_every_file() {
    let t = Tmp::new("disc-any");
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['checks/**/*.mjs'] } }",
    );
    t.file("checks/one.mjs", T);
    t.file("checks/deep/two.mjs", T);
    t.file("checks/three.test.ts", T);
    assert_eq!(
        discover(&t.0, None),
        vec![t.0.join("checks/deep/two.mjs"), t.0.join("checks/one.mjs")]
    );
}

#[test]
fn discover_applies_a_config_exclude_without_include() {
    // turbo-test#60, as vitest 4.1.12: a config exclude applies with the default include
    let t = Tmp::new("disc-excl");
    t.file(
        "vitest.config.ts",
        "export default { test: { exclude: ['**/skip/**'] } }",
    );
    t.file("a.test.ts", T);
    t.file("skip/b.test.ts", T);
    t.file("notatest.ts", T);
    assert_eq!(discover(&t.0, None), vec![t.0.join("a.test.ts")]);
}

#[test]
fn discover_ignores_coverage_block_patterns() {
    // no test.include → default discovery; coverage.exclude's test glob must not exclude tests
    let t = Tmp::new("disc-cov");
    t.file(
        "vitest.config.ts",
        "export default { test: { coverage: { include: ['src/**'], exclude: ['**/*.test.{ts,tsx}'] } } }",
    );
    t.file("src/a.test.ts", T);
    assert_eq!(discover(&t.0, None), vec![t.0.join("src/a.test.ts")]);
    // test.include before the coverage block is used; coverage.exclude still ignored
    let t = Tmp::new("disc-cov2");
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['**/*.test.ts'], coverage: { exclude: ['**/*.test.ts'] } } }",
    );
    t.file("src/a.test.ts", T);
    assert_eq!(discover(&t.0, None), vec![t.0.join("src/a.test.ts")]);
}

// ---- config lookup ------------------------------------------------------------------------

#[test]
fn config_precedence_and_upward_search() {
    let t = Tmp::new("cfg");
    t.file("vite.config.ts", "A");
    t.file("vitest.config.mjs", "B");
    let c = find_config(&t.0, None).unwrap();
    assert_eq!((c.dir.clone(), c.text.as_str()), (t.0.clone(), "B"));
    t.file("vitest.config.ts", "C");
    assert_eq!(find_config(&t.0, None).unwrap().text, "C");
    // found walking up from a nested dir; its dir is the root
    std::fs::create_dir_all(t.0.join("pkg/sub")).unwrap();
    let c = find_config(&t.0.join("pkg/sub"), None).unwrap();
    assert_eq!((c.dir, c.text), (t.0.clone(), "C".to_string()));
    // a nearer config wins
    t.file("pkg/vite.config.js", "D");
    assert_eq!(
        find_config(&t.0.join("pkg/sub"), None).unwrap().dir,
        t.0.join("pkg")
    );
    // a config name that is a directory is skipped
    let t2 = Tmp::new("cfg-dir");
    std::fs::create_dir_all(t2.0.join("vitest.config.ts")).unwrap();
    t2.file("vite.config.mts", "E");
    assert_eq!(find_config(&t2.0, None).unwrap().text, "E");
}

/// turbo-test#49: every config file vitest 4 resolves, in its order (`vitest.config` then
/// `vite.config`, each `.ts .mts .cts .js .mjs .cjs`), as vitest 4.1.12.
#[test]
fn find_config_resolves_every_vitest_config_name() {
    for name in crate::launcher::VITEST_CONFIG_FILES {
        let t = Tmp::new(&format!("cfg-name-{name}"));
        t.file(name, name);
        assert_eq!(find_config(&t.0, None).unwrap().text, name);
    }
    assert_eq!(crate::launcher::VITEST_CONFIG_FILES.len(), 12);
    for (first, second) in [
        ("vitest.config.cjs", "vite.config.ts"),
        ("vitest.config.cts", "vitest.config.js"),
        ("vite.config.cts", "vite.config.js"),
    ] {
        let t = Tmp::new(&format!("cfg-prec-{first}"));
        t.file(first, "first");
        t.file(second, "second");
        assert_eq!(find_config(&t.0, None).unwrap().text, "first", "{first}");
    }
}

#[test]
fn unreadable_config_stops_the_search() {
    let t = Tmp::new("cfg-bad");
    t.file("vite.config.ts", "outer");
    std::fs::create_dir_all(t.0.join("inner")).unwrap();
    std::fs::write(t.0.join("inner/vitest.config.ts"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(find_config(&t.0.join("inner"), None).is_none());
}

#[test]
fn forced_config() {
    let t = Tmp::new("cfg-forced");
    t.file("vitest.config.ts", "root");
    let f = t.file("conf/custom.ts", "forced");
    let c = find_config(&t.0, Some(f.as_path())).unwrap();
    assert_eq!((c.dir, c.text), (t.0.join("conf"), "forced".to_string()));
    assert!(find_config(&t.0, Some(t.0.join("nope.ts").as_path())).is_none());
    assert!(find_config(&t.0, Some(Path::new(""))).is_none());
}

#[test]
fn absolute_in_resolves_against_cwd() {
    let cwd = Path::new("/base/dir");
    assert_eq!(
        absolute_in(cwd, Path::new("a/./b")),
        PathBuf::from("/base/dir/a/b")
    );
    assert_eq!(
        absolute_in(cwd, Path::new("./x")),
        PathBuf::from("/base/dir/x")
    );
    assert_eq!(
        absolute_in(cwd, Path::new("../y")),
        PathBuf::from("/base/dir/../y")
    );
    assert_eq!(
        absolute_in(cwd, Path::new("/abs/./z")),
        PathBuf::from("/abs/z")
    );
    assert_eq!(absolute_in(cwd, Path::new("")), PathBuf::from(""));
}

// ---- string scanners ----------------------------------------------------------------------

#[test]
fn string_array_scanning() {
    let s = "x include y; include : [ 'a', \"b\" , `c`, '' ] exclude:['d']";
    assert_eq!(
        scan_string_array(s, "include").unwrap(),
        vec!["a", "b", "c"]
    );
    assert_eq!(scan_string_array(s, "exclude").unwrap(), vec!["d"]);
    assert!(scan_string_array(s, "missing").is_none());
    assert!(
        scan_string_array("include: [ 'a'", "include").is_none(),
        "unterminated"
    );
    assert!(scan_string_array("include: 'a'", "include").is_none());
    assert!(scan_string_array("include", "include").is_none());
    assert_eq!(
        scan_string_array("include: []", "include").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(extract_quoted("'a' x \"b\" 'unterminated"), vec!["a", "b"]);
    assert_eq!(
        extract_quoted("'a\"b'"),
        vec!["a"],
        "first closing quote of any kind ends it"
    );
}

#[test]
fn environment_scanning() {
    assert_eq!(
        config_environment("test: { environment: 'jsdom' }").as_deref(),
        Some("jsdom")
    );
    assert_eq!(
        config_environment("environment:\"Happy-Dom\"").as_deref(),
        Some("happy-dom")
    );
    assert_eq!(
        config_environment("environment : `node`").as_deref(),
        Some("node")
    );
    // non-matching occurrences are skipped until a real one
    assert_eq!(
        config_environment(
            "environmentOptions: {}, environment: 1, environment: '', environment: 'node'"
        )
        .as_deref(),
        Some("node")
    );
    assert_eq!(config_environment("environment: '1x'"), None);
    assert_eq!(config_environment("environment"), None);
    assert_eq!(config_environment("nothing here"), None);
}

#[test]
fn coverage_block_and_numbers() {
    assert_eq!(
        find_coverage_block("a coverage b coverage : { x }"),
        Some(13)
    );
    assert_eq!(find_coverage_block("coverage: true, coverage"), None);
    assert_eq!(
        scan_braced("t: x, thresholds : { lines: 9 } }", "thresholds").as_deref(),
        Some(" lines: 9 ")
    );
    assert_eq!(
        scan_braced("thresholds: 3, thresholds: { a", "thresholds"),
        None
    );
    assert_eq!(scan_braced("thresholds", "thresholds"), None);
    assert_eq!(
        scan_number(
            "subbranches: 5, x.branches: 6, my_branches: 7, branches: 80.5",
            "branches"
        )
        .as_deref(),
        Some("80.5")
    );
    assert_eq!(scan_number("branches: 'x', branches", "branches"), None);
    assert_eq!(
        scan_number("branches:90", "branches").as_deref(),
        Some("90")
    );
}

#[test]
fn vitest_coverage_config() {
    let t = Tmp::new("covcfg");
    assert!(vitest_coverage(&t.0, None).is_none(), "no config at all");
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['x'] } }",
    );
    let c = vitest_coverage(&t.0, None).unwrap();
    assert!(c.include.is_empty() && c.exclude.is_empty() && c.thresholds.is_none());
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['t/**'], coverage: { include: ['src/**/*.{ts,tsx}'], exclude: ['src/gen/**', 'x.ts'], \
         thresholds: { lines: 90, functions: 80.5, branches: 70, statements: 60 }, lines: 1 } } }",
    );
    let c = vitest_coverage(&t.0, None).unwrap();
    assert_eq!(c.include, vec!["src/**/*.{ts,tsx}"]);
    assert_eq!(c.exclude, vec!["src/gen/**", "x.ts"]);
    assert_eq!(
        c.thresholds.as_deref(),
        Some("lines=90,functions=80.5,branches=70,statements=60")
    );
    // flat legacy thresholds directly under coverage
    t.file(
        "vitest.config.ts",
        "export default { test: { coverage: { provider: 'v8', branches: 50, lines: 40 } } }",
    );
    let c = vitest_coverage(&t.0, None).unwrap();
    assert_eq!(c.thresholds.as_deref(), Some("lines=40,branches=50"));
    t.file(
        "vitest.config.ts",
        "export default { test: { coverage: { enabled: true } } }",
    );
    assert!(vitest_coverage(&t.0, None).unwrap().thresholds.is_none());
}

#[test]
fn vitest_patterns_config() {
    let t = Tmp::new("pats");
    assert!(vitest_patterns(&t.0, None).is_none());
    t.file(
        "vitest.config.ts",
        "export default { test: { exclude: ['a'] } }",
    );
    // no include → the default one (None)
    let p = vitest_patterns(&t.0, None).unwrap();
    assert_eq!(
        (p.root, p.include, p.exclude),
        (t.0.clone(), None, Some(vec!["a".to_string()]))
    );
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['a/**'] } }",
    );
    let p = vitest_patterns(&t.0, None).unwrap();
    assert_eq!(
        (p.root, p.include, p.exclude),
        (t.0.clone(), Some(vec!["a/**".to_string()]), None)
    );
}

// ---- balanced blocks / splitting ----------------------------------------------------------

#[test]
fn balanced_blocks() {
    let s = "x { a: [1, (2)], b: '}', c: \"\\\"}\" } tail";
    let open = s.find('{').unwrap();
    let (inner, end) = balanced_block_end(s, open).unwrap();
    assert_eq!(inner, " a: [1, (2)], b: '}', c: \"\\\"}\" ");
    assert_eq!(&s[end..end + 1], "}");
    assert_eq!(balanced_block_end("[1, [2]]", 0).unwrap().0, "1, [2]");
    assert_eq!(balanced_block_end("(a(b))", 0).unwrap().0, "a(b)");
    assert!(
        balanced_block_end("{ a: [1 }", 0).is_none(),
        "mismatched close is not the end"
    );
    assert!(balanced_block_end("{ 'open", 0).is_none());
    assert!(balanced_block_end("x", 0).is_none(), "not an opener");
}

#[test]
fn top_level_splitting() {
    assert_eq!(
        split_top_level(" a: f(x, y), b: [1,2] , c: '{,}' ,"),
        vec!["a: f(x, y)", "b: [1,2]", "c: '{,}'"]
    );
    assert_eq!(
        split_top_level("a: 'it\\'s, ok', b"),
        vec!["a: 'it\\'s, ok'", "b"]
    );
    assert!(split_top_level("  ").is_empty());
    assert_eq!(
        split_top_level_plus("p(a + b) + '/x+y' + \"\\\"+\""),
        vec!["p(a + b)", "'/x+y'", "\"\\\"+\""]
    );
    assert_eq!(split_top_level_plus(""), vec![""]);
    assert_eq!(top_level_colon("'a:b': x"), Some(5));
    assert_eq!(top_level_colon("[a:b]: x"), Some(5));
    assert_eq!(top_level_colon("'\\':': x"), Some(5));
    assert_eq!(top_level_colon("no colon"), None);
    assert_eq!(top_level_comma("f(a, b), c"), Some(7));
    assert_eq!(top_level_comma("'a,\\'b', c"), Some(7));
    assert_eq!(top_level_comma("[a, b]"), None);
}

#[test]
fn blanking_quoted_spans() {
    assert_eq!(
        blank_quoted("a 'b' \"c\\\"d\" e"),
        format!("a{}e", " ".repeat(12))
    );
    assert_eq!(
        blank_quoted("x 'unterminated\\"),
        format!("x {}", " ".repeat(14))
    );
    assert_eq!(blank_quoted("`t` y"), "    y");
}

#[test]
fn value_after_key_word_boundaries() {
    assert_eq!(
        value_after_key("find: '@', replacement: 'x'", "find").as_deref(),
        Some("'@'")
    );
    assert_eq!(
        value_after_key("refind: 1, find : '@'", "find").as_deref(),
        Some("'@'")
    );
    assert_eq!(
        value_after_key("x.find: 1, $find: 2, _find: 3, find: f(a, b)", "find").as_deref(),
        Some("f(a, b)")
    );
    assert_eq!(value_after_key("find '@'", "find"), None);
    assert_eq!(value_after_key("nothing", "find"), None);
}

// ---- comment / regex stripping -------------------------------------------------------------

#[test]
fn strip_line_and_block_comments() {
    assert_eq!(strip_comments("a // c }\nb"), "a \nb");
    assert_eq!(strip_comments("a // trailing"), "a ");
    assert_eq!(strip_comments("a /* x } */ b"), "a   b");
    assert_eq!(strip_comments("a /* unterminated"), "a  ");
    // comment-like text inside strings survives, escapes included
    assert_eq!(
        strip_comments("'http://x' \"a\\\"//b\" `/*t*/`"),
        "'http://x' \"a\\\"//b\" `/*t*/`"
    );
}

#[test]
fn strip_regex_literals_but_not_division() {
    // regex after `(`: blanked (its quote chars must not leak)
    assert_eq!(
        strip_comments("p.replace(/\"/g, 'x')"),
        "p.replace(   g, 'x')"
    );
    // escaped slash and character class with `/`
    assert_eq!(
        strip_comments("x = /a\\/[/]b/"),
        format!("x = {}", " ".repeat(9))
    );
    // trailing backslash at EOF inside a regex
    assert_eq!(strip_comments("(/a\\"), "(   ");
    // keyword-led regex, after a space
    assert_eq!(strip_comments("return /'/"), "return    ");
    assert_eq!(strip_comments("typeof/x/"), "typeof   ");
    // division after identifiers, numbers and `)` stays
    assert_eq!(strip_comments("a / b / c"), "a / b / c");
    assert_eq!(strip_comments("(1) / 2"), "(1) / 2");
    // a misjudged regex stops at the newline
    assert_eq!(strip_comments("= / 2\nkeep: 'x'"), "=    \nkeep: 'x'");
    // regex at the very start of input
    assert_eq!(strip_comments("/re/ x"), "     x");
    // punctuation after a space clears the keyword memory: `in . / 2` is division
    assert_eq!(strip_comments("in . / 2"), "in . / 2");
    // turbo-test#32 item 8c: a keyword reached by `.` property access is a member NAME, so a
    // `/` after it is division — it blanked the rest of the line as a "regex"
    for src in [
        "x = obj.in / 2, y = 'k'",
        "x = obj?.return / 2",
        "x = a.b.typeof/2",
        "x = obj. in / 2",
        "x = obj.\n  of / 2",
    ] {
        assert_eq!(strip_comments(src), src, "{src}");
    }
    // ... while the same keyword as an operator still opens a regex, even right after a member
    assert_eq!(strip_comments("a.b in /x/"), "a.b in    ");
    assert_eq!(strip_comments("o.in\nreturn /'/"), "o.in\nreturn    ");
}

#[test]
fn regex_context_chars() {
    for c in [
        '\0', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', ';', '+', '-', '*', '/', '%', '^',
        '<', '>', '~',
    ] {
        assert!(is_regex_context(c), "{c:?}");
    }
    for c in ['a', '1', ')', ']', '}', '_', '$', '"'] {
        assert!(!is_regex_context(c), "{c:?}");
    }
}

// ---- aliases -----------------------------------------------------------------------------

#[test]
fn key_blocks() {
    let s = "resolve: { conditions: [] }, x: 'alias: {no}', myalias: { a: 1 }, $alias: [1], alias: { '@': './src' }, test: { alias: [ { find: 'a', replacement: 'b' } ] }, alias: {}";
    assert_eq!(
        find_key_blocks(s, "alias"),
        vec![" '@': './src' ", " { find: 'a', replacement: 'b' } ", ""]
    );
    assert!(find_key_blocks("alias: { unterminated", "alias").is_empty());
    assert!(find_key_blocks("alias = 1; alias: 'x'; 'a\\'lias: {}'", "alias").is_empty());
    assert_eq!(find_key_blocks("alias:{x:1}", "alias"), vec!["x:1"]);
}

#[test]
fn alias_target_resolution() {
    let d = Path::new("/proj/cfg");
    assert_eq!(resolve_alias_target(d, "/abs/x"), "/abs/x");
    assert_eq!(resolve_alias_target(d, "./src//a/./b"), "/proj/cfg/src/a/b");
    assert_eq!(resolve_alias_target(d, "../lib"), "/proj/lib");
    assert_eq!(resolve_alias_target(d, "."), "/proj/cfg");
    assert_eq!(resolve_alias_target(d, ".."), "/proj");
    assert_eq!(resolve_alias_target(d, "preact/compat"), "preact/compat");
    assert_eq!(
        resolve_alias_join(d, &["../..".into(), "pkg/./src".into()]),
        "/pkg/src"
    );
    assert_eq!(
        resolve_alias_join(d, &["a".into(), "/reset".into(), "b".into()]),
        "/reset/b"
    );
    assert_eq!(resolve_alias_join(d, &[]), "/proj/cfg");

    assert_eq!(
        alias_target(d, "path.resolve(__dirname, './src')").as_deref(),
        Some("/proj/cfg/src")
    );
    assert_eq!(
        alias_target(d, "path.join(__dirname, '../..', 'pkg/src')").as_deref(),
        Some("/pkg/src")
    );
    assert_eq!(
        alias_target(d, "fileURLToPath(new URL('./x', import.meta.url))").as_deref(),
        Some("/proj/cfg/x")
    );
    assert_eq!(
        alias_target(d, "new URL('./y', import.meta.url).pathname").as_deref(),
        Some("/proj/cfg/y")
    );
    // turbo-test#32 item 8a: a bare dir-name base concatenated with a literal is that dir
    // (`__dirname + '/z'` ≡ `path.resolve(__dirname, 'z')`); it used to be dropped
    assert_eq!(
        alias_target(d, "__dirname + '/z'").as_deref(),
        Some("/proj/cfg/z")
    );
    assert_eq!(
        alias_target(d, " import.meta.dirname+\"/w/\"").as_deref(),
        Some("/proj/cfg/w/")
    );
    assert_eq!(alias_target(d, "__dirname").as_deref(), Some("/proj/cfg"));
    // any other literal-free base is still unknown
    assert_eq!(alias_target(d, "root + '/z'"), None);
    assert_eq!(alias_target(d, "path.resolve(root) + '/z'"), None);
    assert_eq!(
        alias_target(d, "path.resolve(__dirname, 'src') + '/sub' + '/'").as_deref(),
        Some("/proj/cfg/src/sub/")
    );
    assert_eq!(
        alias_target(d, "'./lit' + `/more`").as_deref(),
        Some("/proj/cfg/lit/more")
    );
    // a literal merely containing a marker is a plain literal
    assert_eq!(
        alias_target(d, "'./path.resolve'").as_deref(),
        Some("/proj/cfg/path.resolve")
    );
    assert_eq!(alias_target(d, "'react'").as_deref(), Some("react"));
    assert_eq!(alias_target(d, "someVar"), None);
    assert_eq!(alias_target(d, "path.resolve(root)"), None);
    assert_eq!(first_quoted("x 'a' 'b'").as_deref(), Some("a"));
    assert_eq!(first_quoted("none"), None);
}

#[test]
fn alias_inner_forms() {
    let d = Path::new("/r");
    let obj = parse_alias_inner(
        " '@': path.resolve(__dirname, './src'), utils: './u', [computed]: './c', /re/: './x', \"q\": 'pkg', noValue, k: other ",
        d,
    );
    assert_eq!(
        obj,
        vec![
            ("@".to_string(), "/r/src".to_string()),
            ("utils".into(), "/r/u".into()),
            ("q".into(), "pkg".into()),
        ]
    );
    let arr = parse_alias_inner(
        " { find: '~', replacement: './lib' }, { find: /^re$/, replacement: './x' }, \
         { find: 'nofind' }, { replacement: './z' }, { find: 'p', replacement: path.resolve(__dirname, 'q') } ",
        d,
    );
    assert_eq!(
        arr,
        vec![
            ("~".to_string(), "/r/lib".to_string()),
            ("p".into(), "/r/q".into())
        ]
    );
    assert!(parse_alias_inner("", d).is_empty());
    // turbo-test#32 item 8b: a QUOTED find is a prefix string whatever its first char — only a
    // regex literal find is skipped (unquoted; config_aliases has blanked it by then)
    let arr = parse_alias_inner(
        " { find: '/@z', replacement: './z' }, { find: \"/x\", replacement: './x' }, \
         { find: /^\\/re/, replacement: './r' }, { find:      , replacement: './b' } ",
        d,
    );
    assert_eq!(
        arr,
        vec![
            ("/@z".to_string(), "/r/z".to_string()),
            ("/x".into(), "/r/x".into())
        ]
    );
}

#[test]
fn alias_on_a_line_with_member_keyword_division() {
    // turbo-test#32 item 8c: `obj.in / 2` read as a regex start and blanked the rest of the line
    let t = Tmp::new("alias-in");
    t.file(
        "vitest.config.ts",
        "const obj = { in: 4 }\n\
         export default { half: obj.in / 2, resolve: { alias: { '@h': './half' } } }\n",
    );
    assert_eq!(
        config_aliases(&t.0, None),
        vec![("@h".to_string(), format!("{}/half", t.0.display()))]
    );
}

#[test]
fn config_aliases_end_to_end() {
    let t = Tmp::new("alias");
    assert!(config_aliases(&t.0, None).is_empty(), "no config");
    t.file(
        "vitest.config.ts",
        "import path from 'path'\n\
         // alias: { '@': './commented-out' }\n\
         /* alias: { '}': 'x' } */\n\
         const strip = (p) => p.replace(/^\\/api\\//, '/').replace(/\"/g, '')\n\
         export default {\n\
           resolve: { conditions: ['browser'] },\n\
           resolve2: { alias: { '@': path.resolve(__dirname, './src'), '#a': './a' } },\n\
           test: { alias: [ { find: '@', replacement: './test-src' }, { find: /^x/, replacement: './y' } ] },\n\
         }\n",
    );
    let got = config_aliases(&t.0, None);
    let root = t.0.display().to_string();
    assert_eq!(
        got,
        vec![
            ("@".to_string(), format!("{root}/test-src")),
            ("#a".to_string(), format!("{root}/a"))
        ]
    );
}

// ---- helpers -----------------------------------------------------------------------------

#[test]
fn flag_helpers() {
    for f in VALUE_FLAGS {
        assert!(flag_takes_value(f), "{f}");
    }
    assert!(!flag_takes_value("--coverage") && !flag_takes_value("--reporter=json"));
    let fwd = args(&[
        "--coverage",
        "--coverage-thresholds=lines=1",
        "--testTimeout",
    ]);
    assert!(has_flag(&fwd, "--coverage-thresholds"));
    assert!(has_flag(&fwd, "--testTimeout"));
    assert!(!has_flag(&fwd, "--coverage-include"));
    assert!(!has_flag(
        &args(&["--coverage-thresholdsX"]),
        "--coverage-thresholds"
    ));
}

// ---- git --changed --------------------------------------------------------------------------

fn git(dir: &Path, a: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(a)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {a:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repo with a.test.ts, b.test.ts, d.test.ts committed; then a modified (unstaged), d modified
/// (staged) and c.test.ts untracked.
fn git_fixture(name: &str) -> Tmp {
    let t = Tmp::new(name);
    git(&t.0, &["init", "-q"]);
    for f in ["a.test.ts", "b.test.ts", "d.test.ts"] {
        t.file(f, T);
    }
    t.file(".gitignore", "ignored.test.ts\n");
    git(&t.0, &["add", "."]);
    git(&t.0, &["commit", "-q", "-m", "init"]);
    t.file("a.test.ts", "// changed\n");
    t.file("d.test.ts", "// staged\n");
    git(&t.0, &["add", "d.test.ts"]);
    t.file("c.test.ts", T);
    t.file("ignored.test.ts", T);
    t
}

#[test]
fn git_changed_sets() {
    let t = git_fixture("git");
    let set = |since: &str| {
        let mut v: Vec<String> = git_changed(since, &t.0)
            .unwrap()
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(set("true"), ["a.test.ts", "c.test.ts", "d.test.ts"]);
    assert_eq!(set(""), ["a.test.ts", "c.test.ts", "d.test.ts"]);
    assert_eq!(set("HEAD"), ["a.test.ts", "c.test.ts", "d.test.ts"]);
    // an unknown ref fails both diffs; untracked files are still reported
    assert_eq!(set("no-such-ref"), ["c.test.ts"]);
    let not_repo = Tmp::new("git-none");
    assert!(git_changed("true", &not_repo.0).is_none());
}

#[test]
fn plan_changed_filters_discovered_files() {
    let t = git_fixture("plan-git");
    let p = run_plan(&t, &["--changed"]);
    assert_eq!(argv(&p), vec!["a.test.ts", "c.test.ts", "d.test.ts"]);
    assert!(p.messages.is_empty());
    // since-ref forms (space + inline)
    let p = run_plan(&t, &["--changed", "no-such-ref"]);
    assert_eq!(argv(&p), vec!["c.test.ts"]);
    let p = run_plan(&t, &["--changed=HEAD", "b.test.ts", "a.test.ts"]);
    assert_eq!(argv(&p), args(&["a.test.ts"]));
    // nothing changed among the candidates → exit 0
    let p = run_plan(&t, &["b.test.ts", "--changed"]);
    assert_eq!(p.outcome, Err(0));
    assert_eq!(
        p.messages,
        vec!["turbo-test: --changed: no changed test files (of 1) — exiting 0."]
    );
}

#[test]
fn plan_changed_outside_a_repo_runs_everything() {
    let t = Tmp::new("plan-nogit");
    t.file("a.test.ts", T);
    let p = run_plan(&t, &["--changed"]);
    assert_eq!(argv(&p), vec!["a.test.ts"]);
    assert_eq!(
        p.messages,
        vec!["turbo-test: --changed: git unavailable / not a repo — running all discovered tests."]
    );
}

// ---- plan() -------------------------------------------------------------------------------

#[test]
fn plan_discovers_and_strips_subcommands() {
    let t = Tmp::new("plan-sub");
    t.file("a.test.ts", T);
    t.file("b.spec.ts", T);
    let all = vec!["a.test.ts", "b.spec.ts"];
    let subs: [&[&str]; 4] = [&[], &["run"], &["watch"], &["dev"]];
    for sub in subs {
        let p = run_plan(&t, sub);
        assert_eq!(argv(&p), all, "{sub:?}");
        assert!(p.env.is_empty() && p.messages.is_empty());
    }
    // only a LEADING subcommand is stripped; elsewhere it is a filter (matching nothing here)
    let p = run_plan(&t, &["a.test.ts", "run"]);
    assert_eq!(argv(&p), vec!["a.test.ts"]);
    assert!(p.messages.is_empty());
}

/// vitest 4.1.12's "No test files found" message with the default include / exclude (`filter`:
/// its `filter: …` line, or empty).
fn no_test_files(code: i32, filter: &str) -> String {
    format!(
        "No test files found, exiting with code {code}\n\n{filter}include: **/*.{{test,spec}}.?(c|m)[jt]s?(x)\nexclude:  **/node_modules/**, **/.git/**"
    )
}

/// turbo-test#59, as vitest 4.1.12: discovered files are named relative to the working
/// directory, as filtered ones are; nothing to run prints vitest's lines (the filters, then
/// the include and exclude in effect).
#[test]
fn discovery_names_and_no_files_message_follow_vitest() {
    let t = Tmp::new("plan-vitest-names");
    t.file("src/u.ts", T);
    let p = run_plan(&t, &[]);
    assert_eq!(p.outcome, Err(1));
    assert_eq!(p.messages, vec![no_test_files(1, "")]);
    let p = run_plan(&t, &["--passWithNoTests"]);
    assert_eq!(p.outcome, Err(0));
    assert_eq!(p.messages, vec![no_test_files(0, "")]);
    let p = run_plan(&t, &["foo", "bar"]);
    assert_eq!(p.messages, vec![no_test_files(1, "filter: foo, bar\n")]);
    // a config's include / exclude are the ones printed
    let cfg = t.file(
        "vitest.config.ts",
        "export default { test: { include: ['src/**/*.check.ts', 'x/*.ts'], exclude: ['**/skip/**'] } }",
    );
    let p = run_plan(&t, &["foo"]);
    assert_eq!(
        p.messages,
        vec!["No test files found, exiting with code 1\n\nfilter: foo\ninclude: src/**/*.check.ts, x/*.ts\nexclude:  **/skip/**"]
    );
    std::fs::remove_file(cfg).unwrap();
    t.file("a.test.ts", T);
    t.file("sub/b.spec.ts", T);
    assert_eq!(argv(&run_plan(&t, &[])), vec!["a.test.ts", "sub/b.spec.ts"]);
}

#[test]
fn plan_no_files_found() {
    let t = Tmp::new("plan-empty");
    let p = run_plan(&t, &[]);
    assert_eq!(p.outcome, Err(1));
    assert_eq!(p.messages, vec![no_test_files(1, "")]);
    let p = run_plan(&t, &["--passWithNoTests"]);
    assert_eq!(p.outcome, Err(0));
    assert_eq!(p.messages, vec![no_test_files(0, "")]);
    // with files, --passWithNoTests reaches the runtime: a file with no tests then passes
    // (turbo-test#32 item 7, vitest semantics)
    let f = t.file("a.test.ts", T).display().to_string();
    let p = run_plan(&t, &["--passWithNoTests", &f]);
    assert_eq!(argv(&p), vec!["a.test.ts"]);
    assert_eq!(p.env, vec![("TURBO_PASS_WITH_NO_TESTS", "1".to_string())]);
}

/// turbo-test#58: positional arguments are vitest filters on the discovered files: a file whose
/// root-relative path contains one (case-insensitively; an absolute one as a prefix; a relative
/// one also from the process cwd) runs; none matching is "No test files found" (vitest 4.1.12).
#[test]
fn plan_filters_like_vitest() {
    let t = Tmp::new("plan-filters");
    t.file("sub/a.test.ts", T);
    t.file("sub/deep/b.test.ts", T);
    t.file("other/data.test.ts", T);
    t.file("notincluded.ts", T);
    // "" args are dropped; filters that match nothing add nothing
    let p = run_plan(
        &t,
        &["sub/a.test.ts", "", "gone.test.ts", "/nope/x.test.ts"],
    );
    assert_eq!(argv(&p), vec!["sub/a.test.ts"]);
    assert!(p.messages.is_empty());
    // a substring, case-insensitively
    assert_eq!(
        argv(&run_plan(&t, &["A.TEST"])),
        vec!["other/data.test.ts", "sub/a.test.ts"]
    );
    assert_eq!(argv(&run_plan(&t, &["deep"])), vec!["sub/deep/b.test.ts"]);
    // a directory, with or without its trailing slash or a leading ./
    let sub = vec!["sub/a.test.ts", "sub/deep/b.test.ts"];
    for f in ["sub", "./sub/", "sub/"] {
        assert_eq!(argv(&run_plan(&t, &[f])), sub, "{f}");
    }
    // an absolute directory
    let abs = t.p("sub");
    assert_eq!(argv(&run_plan(&t, &[&abs])), sub);
    for none in [&["gone.test.ts"][..], &["notincluded.ts"]] {
        let p = run_plan(&t, none);
        assert_eq!(p.outcome, Err(1));
        assert_eq!(
            p.messages,
            vec![no_test_files(1, &format!("filter: {}\n", none[0]))]
        );
    }
    let p = run_plan(&t, &["gone.test.ts", "--passWithNoTests"]);
    assert_eq!(p.outcome, Err(0));
}

#[test]
fn plan_forwards_runner_flags() {
    let t = Tmp::new("plan-fwd");
    let f = t.file("a.test.ts", T);
    let fs = f.display().to_string();
    let p = run_plan(
        &t,
        &[
            "--reporter",
            "json",
            "-j",
            "2",
            "--retry",
            "1",
            "--frob",
            &fs,
            "--silent",
            "-t",
            "adds",
            "--no-file-parallelism",
        ],
    );
    assert_eq!(
        argv(&p),
        args(&[
            "--reporter",
            "json",
            "-j",
            "2",
            "--retry",
            "1",
            "--frob",
            "--silent",
            "-t",
            "adds",
            "--no-file-parallelism",
            "a.test.ts"
        ])
    );
    assert!(p.messages.is_empty(), "{:?}", p.messages);
    // turbo-test#32 item 3: a value flag with NO value (the last token, or followed by another
    // flag) is dropped with a warning. Forwarded alone it sat right before the appended files,
    // so the runner took the first test file as its value (`a.test.js --reporter` → usage
    // error; `-t` → the path became the name pattern).
    for flag in [
        "--outputFile",
        "--reporter",
        "-t",
        "--testNamePattern",
        "--shard",
    ] {
        let p = run_plan(&t, &[&fs, flag]);
        assert_eq!(argv(&p), vec!["a.test.ts".to_string()], "{flag}");
        assert_eq!(
            p.messages,
            vec![format!("turbo-test: ignoring '{flag}': missing value")],
            "{flag}"
        );
    }
    let p = run_plan(&t, &["--bail", "--retry", "1", "-t", "-x", &fs]);
    assert_eq!(argv(&p), args(&["--retry", "1", "-x", "a.test.ts"]));
    assert_eq!(
        p.messages,
        vec![
            "turbo-test: ignoring '--bail': missing value".to_string(),
            "turbo-test: ignoring '-t': missing value".to_string(),
        ]
    );
    // a value-less `--coverage-*` value flag still implies --coverage (as in the runner)
    let p = run_plan(&t, &[&fs, "--coverage-dir"]);
    assert_eq!(argv(&p), args(&["--coverage", "a.test.ts"]));
    assert_eq!(p.messages.len(), 1);
    // inline value forms are forwarded as one token
    assert_eq!(
        argv(&run_plan(&t, &["--reporter=tap", &fs])),
        args(&["--reporter=tap", "a.test.ts"])
    );
}

#[test]
fn plan_drops_noop_vitest_flags() {
    let t = Tmp::new("plan-noop");
    let f = t.file("a.test.ts", T).display().to_string();
    let p = run_plan(
        &t,
        &[
            "--pool",
            "threads",
            "--mode=dev",
            "--project",
            "--exclude",
            "e/**",
            "--maxConcurrency",
            "4",
            "--run",
            "--watch",
            "-w",
            "--no-watch",
            "--logHeapUsage",
            "--hideSkippedTests",
            "--disableConsoleIntercept",
            "--dom",
            "--segfaultRetry",
            "--printConsoleTrace",
            "--color",
            "--no-color",
            "--inspect",
            "--inspect-brk",
            "--globals",
            "--no-globals",
            "--sequence.seed",
            "123",
            "--sequence.shuffle",
            "--browser.name=chromium",
            "--browser",
            "--inspect-wait",
            "9229",
            &f,
        ],
    );
    assert_eq!(argv(&p), vec!["a.test.ts".to_string()]);
    // a value-less no-op flag doesn't eat the following test file
    assert_eq!(
        argv(&run_plan(&t, &["--run", &f])),
        vec!["a.test.ts".to_string()]
    );
    assert_eq!(
        argv(&run_plan(&t, &["--pool", "--run", &f])),
        vec!["a.test.ts".to_string()]
    );
}

#[test]
fn plan_isolate_environment_and_root_flags() {
    let t = Tmp::new("plan-env");
    t.file("proj/a.test.ts", T);
    t.file(
        "proj/vitest.config.ts",
        "export default { test: { environment: 'jsdom', testTimeout: 1234 } }",
    );
    let a = "proj/a.test.ts";
    let p = run_plan(&t, &["--no-isolate", "--root", "proj"]);
    // --root (not --dir) is the base the reports name files from (turbo-test#63 item 2)
    assert_eq!(
        p.env,
        vec![
            ("TURBO_ROOT", t.p("proj")),
            ("TURBO_REUSE_ISOLATE", "1".to_string()),
            ("TURBO_ENV", "jsdom".to_string())
        ]
    );
    assert_eq!(argv(&p), args(&["--testTimeout", "1234", a]));
    // flag beats config; --dir / inline forms
    let p = run_plan(
        &t,
        &[
            "--isolate",
            "--dir=proj",
            "--environment",
            "happy-dom",
            "--testTimeout",
            "9",
        ],
    );
    assert_eq!(
        p.env,
        vec![
            ("TURBO_NO_REUSE", "1".to_string()),
            ("TURBO_ENV", "happy-dom".to_string())
        ]
    );
    assert_eq!(argv(&p), args(&["--testTimeout", "9", a]));
    let p = run_plan(
        &t,
        &["--environment=node", "--root", "proj", "--testTimeout=5"],
    );
    assert_eq!(env_of(&p, "TURBO_ENV"), Some("node"));
    assert_eq!(argv(&p), args(&["--testTimeout=5", a]));
    // --environment with no value (next token is a flag) falls back to the config
    let p = run_plan(&t, &["--environment", "--root", "proj"]);
    assert_eq!(env_of(&p, "TURBO_ENV"), Some("jsdom"));
    // an absolute --root; last of --root/--dir wins
    let p = run_plan(&t, &["--root", "nowhere", "--dir", &t.p("proj")]);
    assert_eq!(argv(&p), args(&["--testTimeout", "1234", a]));
    // a --root without a config: only the naming root, no timeout injection
    t.file("plain/b.test.ts", T);
    let p = run_plan(&t, &["--root", "plain"]);
    assert_eq!(p.env, vec![("TURBO_ROOT", t.p("plain"))]);
    assert_eq!(argv(&p), vec!["plain/b.test.ts"]);
    // `--root ""` is an empty discovery root (nothing found), like the original
    assert_eq!(run_plan(&t, &["--root="]).outcome, Err(1));
    // a value-less trailing --testTimeout is dropped (turbo-test#32 item 3), so it no longer
    // counts as passed: the config's value is injected
    let p = run_plan(&t, &["--root", "proj", "--testTimeout"]);
    assert_eq!(argv(&p), args(&["--testTimeout", "1234", a]));
}

#[test]
fn plan_forced_config() {
    let t = Tmp::new("plan-forced");
    t.file(
        "vitest.config.ts",
        "export default { test: { include: ['nothing/**'], environment: 'node' } }",
    );
    t.file("cfg/my.config.ts", "export default { test: { include: ['**/*.test.ts'], environment: 'jsdom', alias: { '@': './src' } } }");
    t.file("cfg/a.test.ts", T);
    t.file("cfg/sub/b.test.ts", T);
    t.file("cfg/c.spec.ts", T);
    // the forced config's dir is the include root (its include drops c.spec.ts)
    let p = run_plan(&t, &["-c", "cfg/my.config.ts"]);
    assert_eq!(argv(&p), vec!["cfg/a.test.ts", "cfg/sub/b.test.ts"]);
    assert_eq!(env_of(&p, "TURBO_ENV"), Some("jsdom"));
    assert_eq!(
        env_of(&p, "TURBO_ALIAS").unwrap(),
        format!("@\u{1f}{}", t.p("cfg/src"))
    );
    let p = run_plan(&t, &["--config=cfg/my.config.ts"]);
    assert_eq!(env_of(&p, "TURBO_ENV"), Some("jsdom"));
    // without -c the root config applies (nothing included → no files)
    let p = run_plan(&t, &[]);
    assert_eq!(p.outcome, Err(1));
    assert_eq!(env_of(&p, "TURBO_ENV"), Some("node"));
    // a missing forced config means "no config" (default discovery, no env)
    let p = run_plan(&t, &["--config", "missing.ts"]);
    assert_eq!(
        argv(&p),
        vec!["cfg/a.test.ts", "cfg/c.spec.ts", "cfg/sub/b.test.ts"]
    );
    assert!(p.env.is_empty());
    // -c with no value
    let p = run_plan(&t, &["-c", "--passWithNoTests"]);
    assert_eq!(p.outcome, Err(0));
}

#[test]
fn plan_aliases_serialized() {
    let t = Tmp::new("plan-alias");
    t.file("a.test.ts", T);
    t.file(
        "vite.config.ts",
        "export default { resolve: { alias: [ { find: '@', replacement: '/abs/src' }, { find: 'react', replacement: 'preact/compat' } ] } }",
    );
    let p = run_plan(&t, &[]);
    assert_eq!(
        env_of(&p, "TURBO_ALIAS"),
        Some("@\u{1f}/abs/src\u{1e}react\u{1f}preact/compat")
    );
}

#[test]
fn plan_coverage_injection() {
    let t = Tmp::new("plan-cov");
    let f = t.file("a.test.ts", T).display().to_string();
    t.file(
        "vitest.config.ts",
        "export default { test: { coverage: { include: ['src/**'], exclude: ['src/gen/**', 'x.ts'], thresholds: { lines: 90, branches: 80 } } } }",
    );
    // no coverage flag → nothing injected
    assert_eq!(argv(&run_plan(&t, &[&f])), vec!["a.test.ts".to_string()]);
    let p = run_plan(&t, &["--coverage", &f]);
    assert_eq!(
        argv(&p),
        args(&[
            "--coverage",
            "--coverage-thresholds",
            "lines=90,branches=80",
            "--coverage-include",
            "src/**",
            "--coverage-exclude",
            "src/gen/**,x.ts",
            "a.test.ts"
        ])
    );
    // explicit flags win (space, inline and the singular alias)
    let p = run_plan(
        &t,
        &[
            "--coverage-threshold",
            "lines=1",
            "--coverage-include=lib/**",
            "--coverage-exclude",
            "y",
            &f,
        ],
    );
    assert_eq!(
        argv(&p),
        args(&[
            "--coverage-threshold",
            "lines=1",
            "--coverage-include=lib/**",
            "--coverage-exclude",
            "y",
            "a.test.ts"
        ])
    );
    let p = run_plan(
        &t,
        &["--coverage-thresholds=lines=2", "--coverage-dir", "out", &f],
    );
    assert_eq!(
        argv(&p),
        args(&[
            "--coverage-thresholds=lines=2",
            "--coverage-dir",
            "out",
            "--coverage-include",
            "src/**",
            "--coverage-exclude",
            "src/gen/**,x.ts",
            "a.test.ts"
        ])
    );
    // a config without a coverage block injects nothing
    t.file("vitest.config.ts", "export default { test: {} }");
    assert_eq!(
        argv(&run_plan(&t, &["--coverage", &f])),
        args(&["--coverage", "a.test.ts"])
    );
    // no config at all
    std::fs::remove_file(t.0.join("vitest.config.ts")).unwrap();
    assert_eq!(
        argv(&run_plan(&t, &["--coverage", &f])),
        args(&["--coverage", "a.test.ts"])
    );
}

#[test]
fn prepare_returns_argv_for_runnable_input() {
    // `--root` and an absolute file arg make prepare() independent of the test process's cwd;
    // nothing in this input sets env vars, prints or exits
    let t = Tmp::new("prepare");
    let f = t.file("a.test.ts", T).display().to_string();
    let root = t.p("");
    assert_eq!(
        prepare(args(&["run", "--reporter", "dot", "--root", &root, &f])),
        args(&["--reporter", "dot", &f])
    );
}

#[test]
fn config_true_reads_a_boolean_key() {
    // test.includeTaskLocation (turbo-test#63 item 1)
    assert!(config_true(
        "{ test: { includeTaskLocation : true } }",
        "includeTaskLocation"
    ));
    assert!(!config_true(
        "{ test: { includeTaskLocation: false } }",
        "includeTaskLocation"
    ));
    // a mention that is not a key is skipped; no key at all is false
    assert!(config_true(
        "// includeTaskLocation\n{ includeTaskLocation: true }",
        "includeTaskLocation"
    ));
    assert!(!config_true("{ test: {} }", "includeTaskLocation"));
}
