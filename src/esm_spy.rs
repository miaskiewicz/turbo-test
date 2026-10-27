//! Spy-able namespaces on the ESM-graph loader (turbo-test#131). vitest's module runner gives
//! every module a plain exports object, so `vi.spyOn(ns, 'greet')` replaces the export its
//! importers read. V8's module namespace objects are read-only, so on the ESM graph the spy was
//! dropped. An app module (plain JS, after its TS / JSX transform) therefore gets a setter for
//! each of its exports, registered for its own namespace (a self-import) through runtime.js
//! `__ttNsSetter`; `vi.spyOn` / `mockRestore` call it (`__ttNsSet`) where defining the property
//! fails. A setter assigns the export's local binding (`export const` becomes `let`, an
//! anonymous default export gets a binding); a re-export (`export { x } from`, an exported
//! import, `export *`) forwards to the source module's setter.
//!
//! Edits keep every line where it was (the setter goes after the last line), so test locations
//! and source maps still hold.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, ModuleExportName,
    Statement, VariableDeclarationKind,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use std::collections::HashMap;
use std::path::Path;

/// How a setter reaches one export.
enum Target {
    /// assign this module-scope binding
    Local(String),
    /// forward to `ns` (a namespace import of the source module) under this name
    Forward(String, String),
}

/// `code` (the ES module at `file`) with its exports' setter registered, or `None` when it has
/// no export a setter can reach (or does not parse).
pub fn spyable(file: &Path, code: &str) -> Option<String> {
    if !code.contains("export") {
        return None;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs().with_jsx(true)).parse();
    if !parsed.errors.is_empty() {
        return None;
    }
    let body = &parsed.program.body;
    // local name -> (source, imported name) of the module's named / default imports
    let mut imports: HashMap<String, (String, String)> = HashMap::new();
    // top-level `const` declarations by bound name: their keyword's offset
    let mut consts: HashMap<String, u32> = HashMap::new();
    for st in body {
        match st {
            Statement::ImportDeclaration(d) => {
                for s in d.specifiers.iter().flatten() {
                    let (local, imported) = match s {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            (s.local.name.to_string(), s.imported.name().to_string())
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (s.local.name.to_string(), "default".to_string())
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => continue,
                    };
                    imports.insert(local, (d.source.value.to_string(), imported));
                }
            }
            Statement::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Const => {
                for d in &v.declarations {
                    for id in d.id.get_binding_identifiers() {
                        consts.insert(id.name.to_string(), v.span.start);
                    }
                }
            }
            Statement::ExportNamedDeclaration(e) => {
                if let Some(Declaration::VariableDeclaration(v)) = &e.declaration {
                    if v.kind == VariableDeclarationKind::Const {
                        for d in &v.declarations {
                            for id in d.id.get_binding_identifiers() {
                                consts.insert(id.name.to_string(), v.span.start);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut targets: Vec<(String, Target)> = Vec::new();
    let mut stars: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut ns_of = |source: &str| -> String {
        let i = sources.iter().position(|s| s == source).unwrap_or_else(|| {
            sources.push(source.to_string());
            sources.len() - 1
        });
        format!("__tt_ns_{i}")
    };
    // (start, end, replacement), applied back to front
    let mut edits: Vec<(u32, u32, String)> = Vec::new();
    let mut tail = String::new();
    for st in body {
        match st {
            Statement::ExportNamedDeclaration(e) if e.with_clause.is_none() => {
                if let Some(src) = &e.source {
                    for s in &e.specifiers {
                        let ns = ns_of(&src.value);
                        targets.push((
                            s.exported.name().to_string(),
                            Target::Forward(ns, s.local.name().to_string()),
                        ));
                    }
                    continue;
                }
                match &e.declaration {
                    Some(Declaration::VariableDeclaration(v)) => {
                        if !matches!(
                            v.kind,
                            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
                        ) {
                            for d in &v.declarations {
                                for id in d.id.get_binding_identifiers() {
                                    let n = id.name.to_string();
                                    targets.push((n.clone(), Target::Local(n)));
                                }
                            }
                        }
                    }
                    Some(Declaration::FunctionDeclaration(f)) => {
                        if let Some(id) = &f.id {
                            let n = id.name.to_string();
                            targets.push((n.clone(), Target::Local(n)));
                        }
                    }
                    Some(Declaration::ClassDeclaration(c)) => {
                        if let Some(id) = &c.id {
                            let n = id.name.to_string();
                            targets.push((n.clone(), Target::Local(n)));
                        }
                    }
                    _ => {}
                }
                for s in &e.specifiers {
                    let ModuleExportName::IdentifierReference(local) = &s.local else {
                        continue;
                    };
                    let exported = s.exported.name().to_string();
                    let local = local.name.to_string();
                    match imports.get(&local) {
                        Some((source, imported)) => {
                            let ns = ns_of(source);
                            targets.push((exported, Target::Forward(ns, imported.clone())));
                        }
                        None => targets.push((exported, Target::Local(local))),
                    }
                }
            }
            Statement::ExportAllDeclaration(e)
                if e.exported.is_none() && e.with_clause.is_none() =>
            {
                stars.push(ns_of(&e.source.value));
            }
            Statement::ExportDefaultDeclaration(e) => {
                let local = "__tt_default".to_string();
                match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => match &f.id {
                        Some(id) => {
                            targets.push(("default".into(), Target::Local(id.name.to_string())))
                        }
                        None => {
                            let at = f.params.span.start;
                            edits.push((at, at, format!(" {local}")));
                            tail.push_str(&format!(
                                "Object.defineProperty({local}, \"name\", {{ value: \"default\" }});"
                            ));
                            targets.push(("default".into(), Target::Local(local)));
                        }
                    },
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => match &c.id {
                        Some(id) => {
                            targets.push(("default".into(), Target::Local(id.name.to_string())))
                        }
                        None => {
                            let start = c.span.start as usize;
                            let Some(k) = code[start..c.span.end as usize].find("class") else {
                                continue;
                            };
                            let at = (start + k + "class".len()) as u32;
                            edits.push((at, at, format!(" {local}")));
                            tail.push_str(&format!(
                                "Object.defineProperty({local}, \"name\", {{ value: \"default\", configurable: true }});"
                            ));
                            targets.push(("default".into(), Target::Local(local)));
                        }
                    },
                    d => {
                        let Some(expr) = d.as_expression() else {
                            continue;
                        };
                        let (s, x) = (expr.span().start, expr.span().end);
                        // `export default <expr>` -> a binding of its own (an anonymous function
                        // or class keeps the name `default`), exported as `default`
                        edits.push((e.span.start, s, format!("let {local} = ({{ default: ")));
                        edits.push((x, x, " }).default".to_string()));
                        tail.push_str(&format!("export {{ {local} as default }};"));
                        targets.push(("default".into(), Target::Local(local)));
                    }
                }
            }
            _ => {}
        }
    }
    if targets.is_empty() && stars.is_empty() {
        return None;
    }
    // a `const` a setter assigns becomes a `let` (same width: `let  `)
    let mut lets: Vec<u32> = targets
        .iter()
        .filter_map(|(_, t)| match t {
            Target::Local(n) => consts.get(n).copied(),
            Target::Forward(..) => None,
        })
        .collect();
    lets.sort_unstable();
    lets.dedup();
    for at in lets {
        if code[at as usize..].starts_with("const") {
            edits.push((at, at + 5, "let  ".to_string()));
        }
    }
    let mut out = code.to_string();
    edits.sort_by_key(|e| std::cmp::Reverse((e.0, e.1)));
    for (s, e, text) in edits {
        out.replace_range(s as usize..e as usize, &text);
    }
    let me = file.file_name()?.to_string_lossy().into_owned();
    let lit = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut setter = String::new();
    for (i, s) in sources.iter().enumerate() {
        setter.push_str(&format!("import * as __tt_ns_{i} from {};", lit(s)));
    }
    setter.push_str(&format!(
        "import * as __tt_self from {};",
        lit(&format!("./{me}"))
    ));
    setter.push_str(&tail);
    setter.push_str("globalThis.__ttNsSetter?.(__tt_self, (k, v) => { switch (k) {");
    for (name, t) in &targets {
        match t {
            Target::Local(local) => {
                setter.push_str(&format!(" case {}: {local} = v; return true;", lit(name)))
            }
            Target::Forward(ns, imported) => setter.push_str(&format!(
                " case {}: return globalThis.__ttNsSet({ns}, {}, v);",
                lit(name),
                lit(imported)
            )),
        }
    }
    setter.push_str(" } ");
    for ns in &stars {
        setter.push_str(&format!(
            "if (k !== \"default\" && k in {ns}) return globalThis.__ttNsSet({ns}, k, v); "
        ));
    }
    setter.push_str("return false; });");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&setter);
    out.push('\n');
    Some(out)
}

#[cfg(test)]
#[path = "esm_spy_tests.rs"]
mod tests;
