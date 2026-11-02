//! Reads of a mock's missing default on the ES-module graph loader (turbo-test#151).
//!
//! vitest throws `[vitest] No "default" export is defined on the "x" mock…` on every read of the
//! default import of a `vi.mock` factory that returned no `default` — `typeof X`, `if (X)`,
//! `X === y` included. The graph loader serves a mock as a synthetic module, whose export is a
//! plain value: its default is runtime.js's throwing stand-in (`__ttMockDefault`, turbo-test#137),
//! which can only throw when it is used. So a module importing the default of such a mock is
//! compiled with each read of that binding wrapped in `__ttMockRead(…)`, which throws vitest's
//! error for the stand-in ([`guard_default_reads`]). Only modules that import one are rewritten:
//! every other module keeps its source (and its error columns) as is.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Expression, IdentifierReference, ImportDeclarationSpecifier, ModuleExportName, ObjectProperty,
    PropertyKey, Statement,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType};
use std::collections::HashSet;

/// The runtime.js function each read is wrapped in.
const READ: &str = "globalThis.__ttMockRead";

/// `src` (an ES module, plain JavaScript) with every read of a default import from a specifier
/// `guarded` accepts wrapped in `(globalThis.__ttMockRead(…))` (parenthesized: `new X()` stays a
/// construction of the value); `None` when it has none (or does
/// not parse). `import X from 's'` and `import { default as X } from 's'` count; a re-export
/// (`export { X }`) keeps the binding, a shorthand property `{ X }` becomes `{ X: read(X) }`.
pub fn guard_default_reads(src: &str, guarded: &dyn Fn(&str) -> bool) -> Option<String> {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, src, SourceType::mjs()).parse();
    if parsed.panicked || !parsed.errors.is_empty() {
        return None;
    }
    // (builds the bindings' symbol ids too)
    let scoping = SemanticBuilder::new()
        .build(&parsed.program)
        .semantic
        .into_scoping();
    let mut symbols = HashSet::new();
    for stmt in &parsed.program.body {
        let Statement::ImportDeclaration(i) = stmt else {
            continue;
        };
        if !guarded(i.source.value.as_str()) {
            continue;
        }
        for s in i.specifiers.iter().flatten() {
            let local = match s {
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportSpecifier(s)
                    if s.imported.name() == "default" =>
                {
                    &s.local
                }
                _ => continue,
            };
            if let Some(sym) = local.symbol_id.get() {
                symbols.insert(sym);
            }
        }
    }
    if symbols.is_empty() {
        return None;
    }
    let mut reads = Reads {
        scoping: &scoping,
        symbols: &symbols,
        edits: Vec::new(),
    };
    reads.visit_program(&parsed.program);
    if reads.edits.is_empty() {
        return None;
    }
    let mut out = src.to_string();
    reads.edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (start, end, text) in reads.edits {
        out.replace_range(start as usize..end as usize, &text);
    }
    Some(out)
}

struct Reads<'s> {
    scoping: &'s Scoping,
    symbols: &'s HashSet<SymbolId>,
    /// `(start, end, replacement)` byte ranges of `src`
    edits: Vec<(u32, u32, String)>,
}

impl Reads<'_> {
    /// The guarded binding `id` reads, if it is one.
    fn guarded(&self, id: &IdentifierReference) -> bool {
        id.reference_id.get().is_some_and(|rid| {
            let r = self.scoping.get_reference(rid);
            r.is_read() && r.symbol_id().is_some_and(|s| self.symbols.contains(&s))
        })
    }
}

impl<'a> Visit<'a> for Reads<'_> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if self.guarded(id) {
            let span = id.span;
            self.edits
                .push((span.start, span.end, format!("({READ}({}))", id.name)));
        }
    }

    fn visit_object_property(&mut self, prop: &ObjectProperty<'a>) {
        if let (true, Expression::Identifier(id)) = (prop.shorthand, &prop.value) {
            if self.guarded(id) {
                let span = prop.span();
                if let PropertyKey::StaticIdentifier(_) = &prop.key {
                    self.edits
                        .push((span.start, span.end, format!("{0}: {READ}({0})", id.name)));
                }
                return;
            }
        }
        walk::walk_object_property(self, prop);
    }

    fn visit_module_export_name(&mut self, _name: &ModuleExportName<'a>) {
        // `export { X }` / `export { X as y }`: the export stays the binding itself
    }
}

#[cfg(test)]
#[path = "mock_read_tests.rs"]
mod tests;
