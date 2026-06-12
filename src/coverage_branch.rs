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
}

impl<'a> Visit<'a> for Collector {
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

/// Parse `source` (its TS/JSX flavour inferred from `file`'s extension) and return its branch
/// decision points. On a parse error, returns whatever was collected (best-effort).
pub fn extract(file: &Path, source: &str) -> Vec<RawBranch> {
    let st = SourceType::from_path(file).unwrap_or_default();
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, st).parse();
    let mut c = Collector { out: Vec::new() };
    c.visit_program(&ret.program);
    c.out
}
