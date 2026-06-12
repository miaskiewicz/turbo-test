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

impl<'a> Visit<'a> for Collector {
    fn visit_statement(&mut self, s: &Statement<'a>) {
        // Count executable statements only, matching Istanbul/c8. The following are NOT statements
        // (they go in the function map, or are module/type structure) and must be skipped — but we
        // still recurse so the executable statements INSIDE them are counted:
        //   - BlockStatement / EmptyStatement   — container / bare `;`
        //   - Function / Class declarations      — belong to function coverage, not statements
        //   - import / export* declarations      — module structure (the export-wrapped *value*
        //                                          declaration is counted separately, below)
        //   - TS type-only declarations          — erased at runtime, never executed
        let skip = matches!(
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
        );
        if !skip {
            self.stmts.push(s.span().start);
        }
        // Recurse regardless: the executable statements INSIDE a skipped wrapper still count, and
        // the variable declaration inside `export const x = …` is reached here as a statement on
        // its own (so export-const is counted, export-function is not — no special-casing needed).
        walk::walk_statement(self, s);
    }

    fn visit_if_statement(&mut self, s: &IfStatement<'a>) {
        let mut arms = vec![s.consequent.span().start];
        let implicit_else = s.alternate.is_none();
        match &s.alternate {
            Some(a) => arms.push(a.span().start),
            None => arms.push(s.span.end), // implicit else (position only; taken derived)
        }
        self.out.push(RawBranch { decision: s.span.start, arms, implicit_else });
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
            self.out.push(RawBranch { decision: s.span.start, arms, implicit_else: false });
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
    let mut c = Collector { out: Vec::new(), stmts: Vec::new() };
    c.visit_program(&ret.program);
    (c.out, c.stmts)
}

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
