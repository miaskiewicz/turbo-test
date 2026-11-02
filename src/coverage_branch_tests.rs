//! Unit tests for `coverage_branch::extract_all`: which nodes are statements, and the decision
//! points (and arm positions) collected for if/else, `?:`, `&&`/`||`/`??` and `switch`.

use super::*;

/// (decision, arms, implicit_else) with every offset rendered as the source text starting there
/// (first 8 bytes), so expectations read as code rather than numbers.
fn branches(file: &str, src: &str) -> Vec<(String, Vec<String>, bool)> {
    let snip = |o: u32| src[o as usize..].chars().take(8).collect::<String>();
    extract_all(Path::new(file), src)
        .0
        .into_iter()
        .map(|b| {
            (
                snip(b.decision),
                b.arms.iter().map(|&a| snip(a)).collect(),
                b.implicit_else,
            )
        })
        .collect()
}

/// The source text each statement starts with (first 10 bytes).
fn stmts(file: &str, src: &str) -> Vec<String> {
    extract_all(Path::new(file), src)
        .1
        .into_iter()
        .map(|o| src[o as usize..].chars().take(10).collect())
        .collect()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn module_structure_and_type_only_declarations_are_not_statements() {
    let src = "\
import a from 'a';
import fs = require('fs');
export * from 'b';
export { a };
export default function d() { go(); }
export class K { m() { k(); } }
type T = number;
interface I { x: T }
enum E { A }
namespace N { inner(); }
declare module 'm' {}
class C {}
;
{ blockInner(); }
top();
";
    // only the executable statements nested inside the skipped wrappers, plus `top()`
    assert_eq!(
        stmts("m.ts", src),
        s(&[
            "go(); }\nex",
            "k(); } }\nt",
            "inner(); }",
            "blockInner",
            "top();\n"
        ])
    );
}

#[test]
fn if_with_and_without_else_and_else_if_chains() {
    let src = "\
function f(x) {
  if (x) { one(); } else { two(); }
  if (x > 1) three();
  if (a) b(); else if (c) d();
}
";
    assert_eq!(
        branches("f.js", src),
        vec![
            ("if (x) {".into(), s(&["{ one();", "{ two();"]), false),
            // implicit else: the second arm is the position just past the `if` statement
            ("if (x > ".into(), s(&["three();", "\n  if (a"]), true),
            ("if (a) b".into(), s(&["b(); els", "if (c) d"]), false),
            ("if (c) d".into(), s(&["d();\n}\n", "\n}\n"]), true),
        ]
    );
}

#[test]
fn ternaries_logical_operators_and_switches() {
    let src = "\
const t = x ? y : z;
const l = p && q || r;
const n = u ?? v;
switch (k) {
  case 1: one(); break;
  default: other();
}
switch (k) {}
";
    assert_eq!(
        branches("t.js", src),
        vec![
            ("x ? y : ".into(), s(&["y : z;\nc", "z;\nconst"]), false),
            // `(p && q) || r`: the outer `||` first, then the nested `&&`
            ("p && q |".into(), s(&["p && q |", "r;\nconst"]), false),
            ("p && q |".into(), s(&["p && q |", "q || r;\n"]), false),
            ("u ?? v;\n".into(), s(&["u ?? v;\n", "v;\nswitc"]), false),
            ("switch (".into(), s(&["case 1: ", "default:"]), false),
            // an empty `switch` has no arms → no decision point
        ]
    );
}

#[test]
fn statements_inside_branches_and_loops_are_counted() {
    let src = "\
function g(xs) {
  for (let i = 0; i < 2; i++) a();
  while (b()) { c(); }
  do d(); while (0);
  switch (xs) { case 1: e(); }
  try { f(); } catch { h(); }
  return xs ? 1 : 2;
}
";
    assert_eq!(
        stmts("g.ts", src),
        s(&[
            "for (let i",
            "a();\n  whi",
            "while (b()",
            "c(); }\n  d",
            "do d(); wh",
            "d(); while",
            "switch (xs",
            "e(); }\n  t",
            "try { f();",
            "f(); } cat",
            "h(); }\n  r",
            "return xs ",
        ])
    );
}

#[test]
fn jsx_flavour_comes_from_the_extension_and_unknown_extensions_parse_as_js() {
    let src = "const el = ok ? <A /> : <B />;\n";
    assert_eq!(branches("c.tsx", src).len(), 1);
    assert_eq!(stmts("c.tsx", src), s(&["const el ="]));
    // an extension oxc doesn't know falls back to the default (plain JS) source type
    assert_eq!(stmts("x.unknown", "a && b;\n"), s(&["a && b;\n"]));
    assert_eq!(branches("x.unknown", "a && b;\n").len(), 1);
}

#[test]
fn a_parse_error_yields_a_best_effort_result_without_panicking() {
    let (b, st) = extract_all(Path::new("bad.ts"), "if (x) { y(); \n const = ;");
    // oxc returns an empty program on a fatal syntax error: nothing is reported, nothing panics
    assert!(b.is_empty());
    assert!(st.is_empty());
}
