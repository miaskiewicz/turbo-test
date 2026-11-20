//! The test file's own scope for its hoisted `vi.mock` factories (turbo-test#72).
//!
//! vitest hoists `vi.mock` inside the test module, so a factory (and what it returns) closes over
//! the file's top-level bindings and reads them once they are initialised. turbo-test runs the
//! factories earlier, in a mock prepass outside the module. To give them the same bindings, the
//! test file is loaded with a binder statement in front of its code ([`binder`]) that hands the
//! runtime a getter (and, for a `let` / `var`, a setter) per top-level binding the factories name;
//! the prepass runs inside `with (globalThis.__ttScopeOf(file)) { … }` ([`with_scope`]), a proxy
//! that resolves those names through the getters (runtime.js `__ttScopeOf`; one scope per file,
//! setup files included: turbo-test#205). A factory that reads one
//! before the module has run throws `Cannot access '<name>' before initialization`, as a read of
//! a binding in its temporal dead zone does.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, Statement,
    VariableDeclarationKind,
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
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::ImportDeclaration(i) => {
                import_bindings(i, &used, &mut add);
                continue;
            }
            Statement::ExportNamedDeclaration(e) => match &e.declaration {
                Some(d) => d,
                None => continue,
            },
            Statement::ExportDefaultDeclaration(e) => {
                default_binding(&e.declaration, &mut add);
                continue;
            }
            s => match s.as_declaration() {
                Some(d) => d,
                None => continue,
            },
        };
        declaration_bindings(decl, &mut add);
    }
    out
}

/// Add each non-API import's local to `add` when `used` says it is value-referenced.
fn import_bindings(
    i: &oxc_ast::ast::ImportDeclaration,
    used: &dyn Fn(&oxc_ast::ast::BindingIdentifier) -> bool,
    add: &mut dyn FnMut(&str, bool),
) {
    let api = matches!(i.source.value.as_str(), "vitest" | "@jest/globals");
    if api {
        return;
    }
    for s in i.specifiers.iter().flatten() {
        let local = match s {
            ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
        };
        if used(local) {
            add(local.name.as_str(), false);
        }
    }
}

/// The binding an `export default` function / class declares.
fn default_binding(d: &ExportDefaultDeclarationKind, add: &mut dyn FnMut(&str, bool)) {
    match d {
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
}

/// The bindings a top-level declaration declares (`let` / `var` / functions assignable).
fn declaration_bindings(decl: &Declaration, add: &mut dyn FnMut(&str, bool)) {
    match decl {
        Declaration::VariableDeclaration(v) if !v.declare => {
            let assignable = matches!(
                v.kind,
                VariableDeclarationKind::Let | VariableDeclarationKind::Var
            );
            for d in &v.declarations {
                for id in d.id.get_binding_identifiers() {
                    add(id.name.as_str(), assignable);
                }
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
pub fn binder(bindings: &[Binding], file: &Path) -> String {
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
        "globalThis.__ttScopeBind({}, {{ {} }}, {{ {} }});",
        crate::runner::js_string_literal(&file.to_string_lossy()),
        get.join(", "),
        set.join(", ")
    )
}

/// `src` with the binder statement placed where it registers its getters before anything the file
/// loads can run a retained factory (turbo-test#401): right after the first top-level statement
/// that loads nothing and ends its line (the declaration of a `mock…` const, the `jest.mock`
/// call …; a directive prologue stays in front), with a `;` in front, so no position moves; the getters read the bindings
/// live, so one read before its declaration still throws the language's own TDZ error. Without
/// such a statement (the file opens with an import / require, or is one line) it goes on a line
/// of its own at the end, after which every binding has run.
pub fn place_binder(path: &Path, src: &str, binder: &str) -> String {
    match bind_at(path, src) {
        Some(at) => format!("{};{binder}{}", &src[..at], &src[at..]),
        None => format!("{src}\n{binder}"),
    }
}

/// The end of the first top-level statement, before any that loads a module, whose line ends
/// with it.
fn bind_at(path: &Path, src: &str) -> Option<usize> {
    let alloc = Allocator::default();
    let stype = SourceType::from_path(path).unwrap_or_default();
    let parsed = Parser::new(&alloc, src, stype).parse();
    let body = &parsed.program.body;
    bind_point(body, src, body.len())
}

/// [`bind_at`] over the first `upto` statements of `body` (the program `src` parses to).
fn bind_point(body: &[Statement], src: &str, upto: usize) -> Option<usize> {
    use oxc_span::GetSpan;
    let ends_line = |i: usize, end: usize| {
        let next = body
            .get(i + 1)
            .map_or(usize::MAX, |s| s.span().start as usize);
        src[end..].find('\n').is_some_and(|nl| end + nl < next)
    };
    body[..upto]
        .iter()
        .enumerate()
        .take_while(|(_, s)| !loads(s, src))
        .map(|(i, s)| (i, s.span().end as usize))
        .find(|&(i, end)| ends_line(i, end))
        .map(|(_, end)| end)
}

/// The start of every binder statement [`binder`] writes.
const BINDER: &str = "globalThis.__ttScopeBind(";

/// `code` (the test file's CommonJS, as it runs: transformed, its mocks hoisted) with the binder
/// statement [`place_binder`] put in the authored source moved up to where [`bind_at`] would put
/// it in this code (turbo-test#429). A file that opens with an import gets its binder at the end
/// of the source, but its transform runs the mocks' setup (a `mock…` const, the `jest.mock`
/// calls) before the imports, so the binder belongs after the first statement there that loads
/// nothing: then a factory a later import runs (a decorator applied at load) reads the binding.
/// Its old place is blanked and it is appended to a line's end, so no line or column of the code
/// moves (source maps, failure locations). `None`: nothing to move.
pub fn relocate_binder(code: &str) -> Option<String> {
    use oxc_span::GetSpan;
    if !code.contains(BINDER) {
        return None;
    }
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, code, SourceType::cjs()).parse();
    let body = &parsed.program.body;
    let (at_stmt, span) = body.iter().enumerate().find_map(|(i, s)| {
        let span = s.span();
        code[span.start as usize..span.end as usize]
            .starts_with(BINDER)
            .then_some((i, span.start as usize..span.end as usize))
    })?;
    let at = bind_point(body, code, at_stmt)?;
    // (nothing runs between there and here: leave it)
    if code[at..span.start]
        .trim_matches(|c: char| c.is_whitespace() || c == ';')
        .is_empty()
    {
        return None;
    }
    let blank: String = code[span.clone()]
        .chars()
        .map(|c| if c == '\n' { c } else { ' ' })
        .collect();
    Some(format!(
        "{};{}{}{blank}{}",
        &code[..at],
        &code[span.clone()],
        &code[at..span.start],
        &code[span.end..]
    ))
}

/// Whether a top-level statement loads a module: an import, a re-export, or one that calls
/// `require(`.
fn loads(stmt: &Statement, src: &str) -> bool {
    use oxc_span::GetSpan;
    let span = stmt.span();
    match stmt {
        Statement::ImportDeclaration(_) | Statement::ExportAllDeclaration(_) => true,
        Statement::ExportNamedDeclaration(e) if e.source.is_some() => true,
        _ => src[span.start as usize..span.end as usize].contains("require("),
    }
}

/// `code` (sloppy-mode script text) run with the names of `bindings` resolved through the test
/// file's scope; the runtime learns the names first. Each file (the test file, every setup file)
/// has a scope of its own, keyed by its path (turbo-test#205).
pub fn with_scope(code: &str, bindings: &[Binding], file: &Path) -> String {
    let names: Vec<String> = bindings
        .iter()
        .map(|(n, _)| crate::runner::js_string_literal(n))
        .collect();
    let key = crate::runner::js_string_literal(&file.to_string_lossy());
    format!(
        "globalThis.__ttScopeNames({key}, [{}]);\nwith (globalThis.__ttScopeOf({key})) {{\n{code}\n}}",
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
