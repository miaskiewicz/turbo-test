//! The test file's own scope for its hoisted `vi.mock` factories (turbo-test#72).
//!
//! vitest hoists `vi.mock` inside the test module, so a factory (and what it returns) closes over
//! the file's top-level bindings and reads them once they are initialised. turbo-test runs the
//! factories earlier, in a mock prepass outside the module. To give them the same bindings, the
//! test file is loaded with a binder statement in front of its code ([`binder`]) that hands the
//! runtime a getter (and, for a `let` / `var`, a setter) per top-level binding the factories name;
//! the prepass runs inside `with (globalThis.__ttScope) { … }` ([`with_scope`]), a proxy that
//! resolves those names through the getters (runtime.js `__ttScope`). A factory that reads one
//! before the module has run throws `Cannot access '<name>' before initialization`, as a read of
//! a binding in its temporal dead zone does.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier,
    Statement, VariableDeclarationKind,
};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// A top-level binding: its name and whether it can be assigned (`let` / `var` / a function).
pub type Binding = (String, bool);

/// The file's top-level bindings that `factories` (the factory sources) mention, in source order.
/// Imports from the test API (`vitest`, `@jest/globals`, which the prepass provides itself) and
/// imports with no value reference (type-only: naming one in the binder would keep an import
/// the TypeScript transform elides) are left out.
pub fn bindings(path: &Path, src: &str, factories: &str) -> Vec<Binding> {
    let alloc = Allocator::default();
    let stype = SourceType::from_path(path).unwrap_or_default();
    let parsed = Parser::new(&alloc, src, stype).parse();
    let scoping = oxc_semantic::SemanticBuilder::new()
        .build(&parsed.program)
        .semantic
        .into_scoping();
    let used = |id: &oxc_ast::ast::BindingIdentifier| {
        id.symbol_id
            .get()
            .is_some_and(|s| scoping.get_resolved_references(s).any(|r| r.is_value()))
    };
    let mut out: Vec<Binding> = Vec::new();
    let mut add = |name: &str, assignable: bool| {
        if mentions(factories, name) && !out.iter().any(|(n, _)| n == name) {
            out.push((name.to_string(), assignable));
        }
    };
    let pattern = |p: &BindingPattern, assignable: bool, add: &mut dyn FnMut(&str, bool)| {
        for id in p.get_binding_identifiers() {
            add(id.name.as_str(), assignable);
        }
    };
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::ImportDeclaration(i) => {
                let api = matches!(i.source.value.as_str(), "vitest" | "@jest/globals");
                for s in i.specifiers.iter().flatten() {
                    let local = match s {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
                    };
                    if !api && used(local) {
                        add(local.name.as_str(), false);
                    }
                }
                continue;
            }
            Statement::ExportNamedDeclaration(e) => match &e.declaration {
                Some(d) => d,
                None => continue,
            },
            Statement::ExportDefaultDeclaration(e) => {
                match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        if let Some(id) = &f.id {
                            add(id.name.as_str(), true);
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        if let Some(id) = &c.id {
                            add(id.name.as_str(), false);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            s => match s.as_declaration() {
                Some(d) => d,
                None => continue,
            },
        };
        match decl {
            Declaration::VariableDeclaration(v) if !v.declare => {
                let assignable = matches!(
                    v.kind,
                    VariableDeclarationKind::Let | VariableDeclarationKind::Var
                );
                for d in &v.declarations {
                    pattern(&d.id, assignable, &mut add);
                }
            }
            Declaration::FunctionDeclaration(f) if !f.declare => {
                if let Some(id) = &f.id {
                    add(id.name.as_str(), true);
                }
            }
            Declaration::ClassDeclaration(c) if !c.declare => {
                if let Some(id) = &c.id {
                    add(id.name.as_str(), false);
                }
            }
            Declaration::TSEnumDeclaration(e) if !e.declare => add(e.id.name.as_str(), false),
            _ => {}
        }
    }
    out
}

/// Whether `text` mentions `name` as a whole identifier.
fn mentions(text: &str, name: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    text.match_indices(name).any(|(at, _)| {
        !text[..at].chars().next_back().is_some_and(ident)
            && !text[at + name.len()..].chars().next().is_some_and(ident)
    })
}

/// The statement (no newline, so lines keep their numbers) that hands the runtime a getter — and
/// for an assignable binding a setter — per binding.
pub fn binder(bindings: &[Binding]) -> String {
    let get: Vec<String> = bindings
        .iter()
        .map(|(n, _)| format!("{n}: () => {n}"))
        .collect();
    let set: Vec<String> = bindings
        .iter()
        .filter(|(_, a)| *a)
        .map(|(n, _)| format!("{n}: (v) => {{ {n} = v; }}"))
        .collect();
    format!(
        "globalThis.__ttScopeBind({{ {} }}, {{ {} }});",
        get.join(", "),
        set.join(", ")
    )
}

/// `code` (sloppy-mode script text) run with the names of `bindings` resolved through the test
/// file's scope; the runtime learns the names first.
pub fn with_scope(code: &str, bindings: &[Binding]) -> String {
    let names: Vec<String> = bindings
        .iter()
        .map(|(n, _)| crate::runner::js_string_literal(n))
        .collect();
    format!(
        "globalThis.__ttScopeNames([{}]);\nwith (globalThis.__ttScope) {{\n{code}\n}}",
        names.join(", ")
    )
}

/// The `(specifier, factory source)` of every call to one of `callees` (`"vi.mock"`, …) whose
/// first argument is a string literal (or a template literal without substitutions), in source
/// order, read from the AST: calls in comments or strings are not calls, and the factory is its
/// whole argument whatever parentheses its strings hold (turbo-test#79 item 1). A call inside a
/// matched factory is not visited. None when `src` parses as neither TSX nor TypeScript.
pub fn mock_calls(src: &str, callees: &[&str]) -> Option<Vec<(String, String)>> {
    for source_type in [SourceType::tsx(), SourceType::ts()] {
        let alloc = Allocator::default();
        let parsed = Parser::new(&alloc, src, source_type).parse();
        if !parsed.errors.is_empty() {
            continue;
        }
        let mut calls = MockCalls {
            src,
            callees,
            out: Vec::new(),
        };
        oxc_ast_visit::Visit::visit_program(&mut calls, &parsed.program);
        return Some(calls.out);
    }
    None
}

struct MockCalls<'s> {
    src: &'s str,
    callees: &'s [&'s str],
    out: Vec<(String, String)>,
}

impl<'a> oxc_ast_visit::Visit<'a> for MockCalls<'_> {
    fn visit_call_expression(&mut self, it: &oxc_ast::ast::CallExpression<'a>) {
        use oxc_ast::ast::{Argument, Expression};
        use oxc_span::GetSpan;
        let callee = match &it.callee {
            Expression::StaticMemberExpression(m) => match &m.object {
                Expression::Identifier(o) => Some(format!("{}.{}", o.name, m.property.name)),
                _ => None,
            },
            _ => None,
        };
        let spec = match it.arguments.first() {
            Some(Argument::StringLiteral(s)) => Some(s.value.to_string()),
            Some(Argument::TemplateLiteral(t)) if t.expressions.is_empty() => t
                .quasis
                .first()
                .map(|q| q.value.cooked.unwrap_or(q.value.raw).to_string()),
            _ => None,
        };
        match (callee, spec) {
            (Some(callee), Some(spec)) if self.callees.contains(&callee.as_str()) => {
                let factory = it.arguments.get(1).map_or("", |f| {
                    let span = f.span();
                    &self.src[span.start as usize..span.end as usize]
                });
                self.out.push((spec, factory.to_string()));
            }
            _ => oxc_ast_visit::walk::walk_call_expression(self, it),
        }
    }
}

#[cfg(test)]
#[path = "mock_scope_tests.rs"]
mod tests;
