//! Spy-able namespaces on the ESM-graph loader (turbo-test#131, #146). vitest's module runner
//! gives every module a plain exports object and compiles an importer's uses of an imported
//! binding to reads of that object, so `vi.spyOn(ns, 'greet')` changes what the importers see,
//! while the exporting module's own references keep the original. V8's module namespaces are
//! read-only, so an app module on the ESM graph gets the same shape by rewriting (plain JS,
//! after its TS / JSX transform):
//!
//! - as an exporter, it registers its namespace (a self-import) with runtime.js
//!   `__ttNsRegister`, naming its own exports and its re-exports (`export { x } from`, an
//!   exported import, `export *`), which read through to the source module;
//! - as an importer, every read of an imported binding becomes `__ttR(<namespace>, <name>)` (the
//!   spied value when there is one, else the live binding), a namespace import is read through
//!   `__ttNs` (a view with the same overrides), and `import()` resolves to that view.
//!
//! `vi.spyOn` / `mockRestore` set the override (`__ttNsSet`). Edits keep every line where it was
//! (the registration follows the last line), so test locations and source maps still hold.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, Expression, IdentifierReference, ImportDeclarationSpecifier, ImportExpression,
    ModuleExportName, ObjectProperty, Statement, VariableDeclarationKind,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc_span::SourceType;
use std::collections::HashMap;
use std::path::Path;

/// `code` (the ES module at `file`) with its imports read through the spy overrides and its
/// namespace registered, or `None` when there is nothing to do (or it does not parse).
/// The returned shifts are the in-line edits (location.rs maps V8's columns back through them).
pub fn spyable(file: &Path, code: &str) -> Option<(String, Vec<crate::location::Shift>)> {
    if !code.contains("import") && !code.contains("export") {
        return None;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
    if !parsed.errors.is_empty() {
        return None;
    }
    let program = parsed.program;
    let semantic = SemanticBuilder::new().build(&program).semantic;
    let scoping = semantic.scoping();
    let lit = |s: &str| serde_json::to_string(s).unwrap_or_default();

    let mut sources: Vec<String> = Vec::new();
    let mut ns_of = |source: &str| -> String {
        let i = sources.iter().position(|s| s == source).unwrap_or_else(|| {
            sources.push(source.to_string());
            sources.len() - 1
        });
        format!("__tt_ns_{i}")
    };
    // import binding symbol -> the expression an importer's read of it becomes
    let mut reads: HashMap<SymbolId, String> = HashMap::new();
    // local name -> (source, imported name) of named / default imports (for re-exports)
    let mut imported: HashMap<String, (String, String)> = HashMap::new();
    let mut own: Vec<String> = Vec::new();
    let mut reexports: Vec<(String, String, String)> = Vec::new();
    let mut stars: Vec<String> = Vec::new();
    for st in &program.body {
        match st {
            Statement::ImportDeclaration(d) if d.with_clause.is_none() && d.phase.is_none() => {
                let source = d.source.value.as_str();
                let api =
                    matches!(source, "vitest" | "@jest/globals") || source.starts_with("node:");
                for s in d.specifiers.iter().flatten() {
                    let (local, name) = match s {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            (&s.local, Some(s.imported.name().to_string()))
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (&s.local, Some("default".to_string()))
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (&s.local, None),
                    };
                    if let Some(n) = &name {
                        imported.insert(local.name.to_string(), (source.to_string(), n.clone()));
                    }
                    // (the semantic pass gives every binding a symbol, a redeclared one too)
                    if let (Some(symbol), false) = (local.symbol_id.get(), api) {
                        let read = match name {
                            Some(n) => {
                                format!("(globalThis.__ttR({}, {}))", ns_of(source), lit(&n))
                            }
                            None => format!("(globalThis.__ttNs({}))", local.name),
                        };
                        reads.insert(symbol, read);
                    }
                }
            }
            Statement::ExportNamedDeclaration(e) if e.with_clause.is_none() => {
                if let Some(src) = &e.source {
                    for s in &e.specifiers {
                        reexports.push((
                            s.exported.name().to_string(),
                            src.value.to_string(),
                            s.local.name().to_string(),
                        ));
                    }
                    continue;
                }
                match &e.declaration {
                    Some(Declaration::VariableDeclaration(v))
                        if !matches!(
                            v.kind,
                            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
                        ) =>
                    {
                        for d in &v.declarations {
                            own.extend(
                                d.id.get_binding_identifiers()
                                    .iter()
                                    .map(|i| i.name.to_string()),
                            );
                        }
                    }
                    Some(Declaration::FunctionDeclaration(f)) => {
                        own.extend(f.id.iter().map(|i| i.name.to_string()))
                    }
                    Some(Declaration::ClassDeclaration(c)) => {
                        own.extend(c.id.iter().map(|i| i.name.to_string()))
                    }
                    _ => {}
                }
                for s in &e.specifiers {
                    let exported = s.exported.name().to_string();
                    match &s.local {
                        ModuleExportName::IdentifierReference(r)
                            if imported.contains_key(r.name.as_str()) =>
                        {
                            let (source, name) = imported[r.name.as_str()].clone();
                            reexports.push((exported, source, name));
                        }
                        _ => own.push(exported),
                    }
                }
            }
            Statement::ExportDefaultDeclaration(_) => own.push("default".to_string()),
            Statement::ExportAllDeclaration(e) if e.with_clause.is_none() => match &e.exported {
                Some(n) => own.push(n.name().to_string()),
                None => stars.push(e.source.value.to_string()),
            },
            _ => {}
        }
    }

    struct Reads<'s> {
        scoping: &'s Scoping,
        reads: &'s HashMap<SymbolId, String>,
        edits: Vec<(u32, u32, String)>,
    }
    impl Reads<'_> {
        fn read_of(&self, it: &IdentifierReference) -> Option<&String> {
            let r = self.scoping.get_reference(it.reference_id.get()?);
            if r.is_write() {
                return None;
            }
            self.reads.get(&r.symbol_id()?)
        }
    }
    impl<'a> Visit<'a> for Reads<'_> {
        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            if let Some(read) = self.read_of(it) {
                self.edits.push((it.span.start, it.span.end, read.clone()));
            }
        }
        fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
            if let (true, Expression::Identifier(id)) = (it.shorthand, &it.value) {
                if let Some(read) = self.read_of(id) {
                    let edit = format!("{}: {read}", id.name);
                    self.edits.push((it.span.start, it.span.end, edit));
                    return;
                }
            }
            walk::walk_object_property(self, it);
        }
        // `export { binding }` names the binding, it does not read it
        fn visit_export_named_declaration(
            &mut self,
            it: &oxc_ast::ast::ExportNamedDeclaration<'a>,
        ) {
            if let Some(d) = &it.declaration {
                self.visit_declaration(d);
            }
        }
        fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
            self.edits
                .push((it.span.start, it.span.start, "(".to_string()));
            let end = it.span.end;
            self.edits
                .push((end, end, ").then(globalThis.__ttNs)".to_string()));
            walk::walk_import_expression(self, it);
        }
    }
    let mut v = Reads {
        scoping,
        reads: &reads,
        edits: Vec::new(),
    };
    v.visit_program(&program);
    let mut edits = v.edits;
    let exporter = !own.is_empty() || !reexports.is_empty() || !stars.is_empty();
    if edits.is_empty() && !exporter {
        return None;
    }
    let mut out = code.to_string();
    edits.sort_by_key(|e| (e.0, e.1));
    let shifts = shifts(code, &edits);
    for (s, e, text) in edits.into_iter().rev() {
        out.replace_range(s as usize..e as usize, &text);
    }
    // the registration, after the last line
    let mut tail = String::new();
    let re: Vec<String> = reexports
        .iter()
        .map(|(name, source, imported)| {
            format!("{}: [{}, {}]", lit(name), ns_of(source), lit(imported))
        })
        .collect();
    let star_ns: Vec<String> = stars.iter().map(|s| ns_of(s)).collect();
    for (i, s) in sources.iter().enumerate() {
        tail.push_str(&format!("import * as __tt_ns_{i} from {};", lit(s)));
    }
    if exporter {
        let me = file.file_name()?.to_string_lossy().into_owned();
        let own: Vec<String> = own.iter().map(|n| lit(n)).collect();
        tail.push_str(&format!(
            "import * as __tt_self from {};globalThis.__ttNsRegister?.(__tt_self, [{}], {{{}}}, [{}]);",
            lit(&format!("./{me}")),
            own.join(", "),
            re.join(", "),
            star_ns.join(", ")
        ));
    }
    if !tail.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&tail);
        out.push('\n');
    }
    Some((out, shifts))
}

/// The edits (sorted, byte offsets into `code`, no newlines) as UTF-16 column shifts per line;
/// a replaced reference that is called (`boom(`, `boom (`) takes in the `(`, so V8's call position maps to the callee.
fn shifts(code: &str, edits: &[(u32, u32, String)]) -> Vec<crate::location::Shift> {
    let mut out = Vec::new();
    // (line, the line's start offset, the column delta so far on it)
    let mut cur: Option<(u32, usize, i64)> = None;
    for (s, e, text) in edits {
        let (s, e) = (*s as usize, *e as usize);
        let line_start = code[..s].rfind('\n').map_or(0, |n| n + 1);
        let line = code[..s].matches('\n').count() as u32;
        let delta = match cur {
            Some((l, _, d)) if l == line => d,
            _ => 0,
        };
        let col = |at: usize| code[line_start..at].encode_utf16().count() as i64;
        // a call of the rewritten reference: V8 places the call at its `(`, vitest's frame at
        // the callee, so the `(` (and blanks before it) belong to the edit (turbo-test#176)
        let rest = &code[e..];
        let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let paren = if s < e && rest[blanks..].starts_with('(') {
            blanks as i64 + 1
        } else {
            0
        };
        let (old_s, old_e) = (col(s), col(e) + paren);
        let new_s = old_s + delta;
        let new_e = new_s + text.encode_utf16().count() as i64 + paren;
        out.push(crate::location::Shift {
            line,
            new: (new_s as u32, new_e as u32),
            old: (old_s as u32, old_e as u32),
        });
        cur = Some((line, line_start, delta + (new_e - new_s) - (old_e - old_s)));
    }
    out
}

#[cfg(test)]
#[path = "esm_spy_tests.rs"]
mod tests;
