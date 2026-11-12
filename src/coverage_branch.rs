//! Branch-point extraction for coverage. Parses the ORIGINAL source with oxc and collects the
//! decision points (if/else, ?:, &&/||/??, switch) as byte spans. The runner then correlates each
//! arm's source position with V8's block-coverage counts (mapped from generated → original) to
//! produce lcov `BRDA`/`BRF`/`BRH`. See `coverage.rs::map_branches`.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// One decision point: byte offset of the decision + each arm's start byte offset (original src).
/// `implicit_else` marks an `if` with no `else` — its last arm has no real block, so its taken
/// count is derived as (block executions − then-taken) rather than sampled at a position.
pub struct RawBranch {
    pub decision: u32,
    pub arms: Vec<u32>,
    pub implicit_else: bool,
}

struct Collector {
    out: Vec<RawBranch>,
    stmts: Vec<u32>, // executable statement starts (Istanbul/c8 "statements")
}

/// True for statement kinds that are NOT executable statements by Istanbul/c8's rule (container /
/// declaration / module-structure / TS type-only), so they never get a statement counter. The
/// executable statements INSIDE them are still visited and counted by `visit_statement`'s recurse.
fn skip_as_statement(s: &Statement<'_>) -> bool {
    matches!(
        s,
        Statement::BlockStatement(_)
            | Statement::EmptyStatement(_)
            | Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportDefaultDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSEnumDeclaration(_)
            | Statement::TSModuleDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
    )
}

impl<'a> Visit<'a> for Collector {
    fn visit_statement(&mut self, s: &Statement<'a>) {
        // Count executable statements only, matching Istanbul/c8. Container / declaration /
        // module-structure / TS type-only kinds are skipped by `skip_as_statement`, but we still
        // recurse so executable statements INSIDE them are counted: the variable declaration inside
        // `export const x = …` is reached here as a statement on its own, so export-const is
        // counted while export-function is not — no special-casing needed.
        if !skip_as_statement(s) {
            self.stmts.push(s.span().start);
        }
        walk::walk_statement(self, s);
    }

    fn visit_if_statement(&mut self, s: &IfStatement<'a>) {
        let mut arms = vec![s.consequent.span().start];
        let implicit_else = s.alternate.is_none();
        match &s.alternate {
            Some(a) => arms.push(a.span().start),
            None => arms.push(s.span.end), // implicit else (position only; taken derived)
        }
        self.out.push(RawBranch {
            decision: s.span.start,
            arms,
            implicit_else,
        });
        walk::walk_if_statement(self, s);
    }

    fn visit_conditional_expression(&mut self, e: &ConditionalExpression<'a>) {
        self.out.push(RawBranch {
            decision: e.span.start,
            arms: vec![e.consequent.span().start, e.alternate.span().start],
            implicit_else: false,
        });
        walk::walk_conditional_expression(self, e);
    }

    fn visit_logical_expression(&mut self, e: &LogicalExpression<'a>) {
        // both operands are branch arms (left short-circuits whether right runs).
        self.out.push(RawBranch {
            decision: e.span.start,
            arms: vec![e.left.span().start, e.right.span().start],
            implicit_else: false,
        });
        walk::walk_logical_expression(self, e);
    }

    fn visit_switch_statement(&mut self, s: &SwitchStatement<'a>) {
        let arms: Vec<u32> = s.cases.iter().map(|c| c.span.start).collect();
        if !arms.is_empty() {
            self.out.push(RawBranch {
                decision: s.span.start,
                arms,
                implicit_else: false,
            });
        }
        walk::walk_switch_statement(self, s);
    }
}

/// Parse `source` (its TS/JSX flavour inferred from `file`'s extension) ONCE and return both its
/// branch decision points and the byte offsets of its executable statements. One parse + one walk
/// feeds both metrics — keeping coverage's per-file AST cost flat as metrics are added. On a parse
/// error, returns whatever was collected (best-effort).
pub fn extract_all(file: &Path, source: &str) -> (Vec<RawBranch>, Vec<u32>) {
    let st = SourceType::from_path(file).unwrap_or_default();
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, st).parse();
    let mut c = Collector {
        out: Vec::new(),
        stmts: Vec::new(),
    };
    c.visit_program(&ret.program);
    (c.out, c.stmts)
}

/// The start offset and name of each function of `source` that V8 names — declarations, class
/// and object methods (by key; `get x` / `set x` for accessors) and functions assigned to a
/// variable, property, class field or member (`exports.h`) — as the loaded path's function
/// coverage counts them: a never-loaded file's functions, for `--coverage-all`. An inline
/// callback V8 leaves unnamed is not counted there, nor here.
pub fn function_starts(file: &Path, source: &str) -> Vec<(u32, String)> {
    let st = SourceType::from_path(file).unwrap_or_default();
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, st).parse();
    let mut fns = Fns {
        source,
        out: Vec::new(),
        pending: None,
    };
    fns.visit_program(&ret.program);
    fns.out
}

struct Fns<'s> {
    source: &'s str,
    out: Vec<(u32, String)>,
    /// the name V8 infers for the function visited next (its key / binding / target)
    pending: Option<String>,
}

impl Fns<'_> {
    fn push(&mut self, start: u32, own: Option<&str>) {
        let name = own.map(str::to_string).or_else(|| self.pending.take());
        self.pending = None;
        if let Some(name) = name {
            self.out.push((start, name));
        }
    }

    /// Name the next function `name` when `value` is one.
    fn name_next(&mut self, value: Option<&Expression>, name: Option<String>) {
        if value.is_some_and(|v| v.without_parentheses().is_function()) {
            self.pending = name;
        }
    }
}

impl<'a> Visit<'a> for Fns<'_> {
    fn visit_method_definition(&mut self, m: &MethodDefinition<'a>) {
        let key = m.key.static_name().map(|k| match m.kind {
            MethodDefinitionKind::Get => format!("get {k}"),
            MethodDefinitionKind::Set => format!("set {k}"),
            _ => k.to_string(),
        });
        self.pending = key;
        walk::walk_method_definition(self, m);
    }
    fn visit_object_property(&mut self, p: &ObjectProperty<'a>) {
        self.name_next(Some(&p.value), p.key.static_name().map(|k| k.to_string()));
        walk::walk_object_property(self, p);
    }
    fn visit_property_definition(&mut self, p: &PropertyDefinition<'a>) {
        self.name_next(p.value.as_ref(), p.key.static_name().map(|k| k.to_string()));
        walk::walk_property_definition(self, p);
    }
    fn visit_variable_declarator(&mut self, d: &VariableDeclarator<'a>) {
        let name = d.id.get_identifier_name().map(|n| n.to_string());
        self.name_next(d.init.as_ref(), name);
        walk::walk_variable_declarator(self, d);
    }
    fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
        let target = &self.source[a.left.span().start as usize..a.left.span().end as usize];
        self.name_next(Some(&a.right), Some(target.to_string()));
        walk::walk_assignment_expression(self, a);
    }
    fn visit_function(&mut self, f: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.push(f.span.start, f.id.as_ref().map(|id| id.name.as_str()));
        walk::walk_function(self, f, flags);
    }
    fn visit_arrow_function_expression(&mut self, a: &ArrowFunctionExpression<'a>) {
        self.push(a.span.start, None);
        walk::walk_arrow_function_expression(self, a);
    }
}

#[cfg(test)]
#[path = "coverage_branch_tests.rs"]
mod unit_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Statement offsets extracted from `src` (a `.ts` module), mapped to the 1-based line of each.
    fn stmt_lines(src: &str) -> Vec<u32> {
        let (_, stmts) = extract_all(Path::new("t.ts"), src);
        stmts
            .into_iter()
            .map(|off| src[..off as usize].bytes().filter(|&b| b == b'\n').count() as u32 + 1)
            .collect()
    }

    #[test]
    fn counts_executable_statements_not_declarations() {
        // Matches Istanbul/c8: the function/export declarations are NOT statements; the body
        // statements are. (Regression: v0.2.6/0.2.7 counted the `export function` lines too,
        // inflating the total and deflating the percentage.)
        let src = "\
export function add(a, b) {
  const sum = a + b;
  if (sum > 10) {
    return 'big';
  }
  return sum;
}
export function unused(x) {
  const y = x * 2;
  return y;
}
";
        // expected statements: const sum(2), if(3), return 'big'(4), return sum(6), const y(9), return y(10)
        assert_eq!(stmt_lines(src), vec![2, 3, 4, 6, 9, 10]);
    }

    #[test]
    fn export_const_is_a_statement_but_export_function_is_not() {
        let src = "\
import dep from 'dep';
export const double = (n) => n * 2;
export function f() { return 1; }
const local = 5;
";
        // import: no; export const double: yes (line 2); export function f: no, but its body
        // `return 1` (line 3) yes; const local: yes (line 4).
        assert_eq!(stmt_lines(src), vec![2, 3, 4]);
    }

    #[test]
    fn skips_block_and_empty_counts_loops_and_throws() {
        let src = "\
function g(xs) {
  let total = 0;
  for (const x of xs) {
    total += x;
  }
  if (!xs.length) throw new Error('empty');
  return total;
}
";
        // let total(2), for-of(3), total += x(4), if(6), throw(6), return(7). No Block/Empty/fn-decl.
        assert_eq!(stmt_lines(src), vec![2, 3, 4, 6, 6, 7]);
    }
}
