//! vite's `import.meta.glob` transform (turbo-test#130), as far as vitest 4.1.12 supports it in
//! tests: a call by the full name with literal arguments is replaced, before any other
//! transform, by an object of the matching modules — eager (`{ eager: true }`: the module
//! namespaces, imported statically) or lazy (`() => import(...)`), each value the module or its
//! `import` export. Patterns are string literals (or an array of them; `!` negates) relative to
//! the module (`./`, `../`) or the project root (`/`); keys are the paths as the pattern writes
//! them, sorted. As vite: the module itself, dot files and `node_modules` are skipped unless
//! `exhaustive: true`. A call it cannot replace (another option, a non-literal argument, an
//! alias pattern) is left alone and throws vitest's error at run time (runtime.js).
//!
//! The replacement keeps the call's line count and the added imports go after the last line, so
//! the module's line numbers (test locations, source maps) do not move.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpressionElement, CallExpression, Expression, ObjectPropertyKind, PropertyKey,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::path::{Component, Path, PathBuf};

/// One `import.meta.glob(...)` call's literal arguments.
#[derive(Debug, PartialEq)]
struct Call {
    span: (u32, u32),
    patterns: Vec<String>,
    eager: bool,
    import: Option<String>,
    exhaustive: bool,
}

/// `src` (the module at `file`) with every replaceable `import.meta.glob` call expanded against
/// the file system, `root` the base of `/` patterns; `None` when there is none.
pub fn expand(file: &Path, src: &str, root: &Path) -> Option<String> {
    if !src.contains("glob") || !src.contains("import.meta") {
        return None;
    }
    let calls = glob_calls(file, src);
    if calls.is_empty() {
        return None;
    }
    let dir = file.parent()?;
    let mut out = src.to_string();
    let mut imports = String::new();
    let mut changed = false;
    for (i, call) in calls.iter().enumerate().rev() {
        let Some(found) = matches(call, file, dir, root) else {
            continue;
        };
        let mut entries = Vec::new();
        for (j, (key, spec)) in found.iter().enumerate() {
            let spec = serde_json::to_string(spec).ok()?;
            let pick = call
                .import
                .as_ref()
                .map(|n| format!("[{}]", serde_json::to_string(n).unwrap_or_default()))
                .unwrap_or_default();
            let value = if call.eager {
                let local = format!("__tt_glob_{i}_{j}");
                imports.push_str(&format!("import * as {local} from {spec};"));
                format!("{local}{pick}")
            } else if pick.is_empty() {
                format!("() => import({spec})")
            } else {
                format!("() => import({spec}).then((m) => m{pick})")
            };
            entries.push(format!("{}: {value}", serde_json::to_string(key).ok()?));
        }
        let (start, end) = (call.span.0 as usize, call.span.1 as usize);
        let lines = "\n".repeat(src[start..end].matches('\n').count());
        let replacement = format!("({{{}}}{lines})", entries.join(", "));
        out.replace_range(start..end, &replacement);
        changed = true;
    }
    if !changed {
        return None;
    }
    if !imports.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&imports);
        out.push('\n');
    }
    Some(out)
}

/// The replaceable `import.meta.glob(...)` calls of `src`, in source order.
fn glob_calls(file: &Path, src: &str) -> Vec<Call> {
    struct Finder(Vec<Call>);
    impl<'a> Visit<'a> for Finder {
        fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
            if is_import_meta_glob(&it.callee) {
                if let Some(call) = literal_call(it) {
                    self.0.push(call);
                }
            }
            walk::walk_call_expression(self, it);
        }
    }
    let allocator = Allocator::default();
    let mut stype = SourceType::from_path(file).unwrap_or_default();
    if !stype.is_typescript() {
        stype = stype.with_jsx(true);
    }
    let parsed = Parser::new(&allocator, src, stype).parse();
    if !parsed.errors.is_empty() {
        return Vec::new();
    }
    let mut f = Finder(Vec::new());
    f.visit_program(&parsed.program);
    f.0
}

/// `import.meta.glob`, the `import.meta` possibly parenthesized or cast (`(import.meta as any)`
/// — esbuild strips the cast before vite sees it).
fn is_import_meta_glob(callee: &Expression) -> bool {
    let Expression::StaticMemberExpression(m) = callee.without_parentheses() else {
        return false;
    };
    let mut object = &m.object;
    loop {
        object = match object {
            Expression::ParenthesizedExpression(p) => &p.expression,
            Expression::TSAsExpression(e) => &e.expression,
            Expression::TSSatisfiesExpression(e) => &e.expression,
            Expression::TSNonNullExpression(e) => &e.expression,
            Expression::TSTypeAssertion(e) => &e.expression,
            _ => break,
        };
    }
    m.property.name == "glob"
        && matches!(object, Expression::MetaProperty(p) if p.meta.name == "import" && p.property.name == "meta")
}

/// The call's patterns and options when they are all literals vite's transform takes.
fn literal_call(it: &CallExpression) -> Option<Call> {
    let text = |e: &Expression| -> Option<String> {
        match e {
            Expression::StringLiteral(s) => Some(s.value.to_string()),
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                Some(t.quasis.first()?.value.cooked?.to_string())
            }
            _ => None,
        }
    };
    let mut call = Call {
        span: (it.span.start, it.span.end),
        patterns: Vec::new(),
        eager: false,
        import: None,
        exhaustive: false,
    };
    let mut args = it.arguments.iter();
    match args.next()?.as_expression()? {
        Expression::ArrayExpression(a) => {
            for el in &a.elements {
                if matches!(
                    el,
                    ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
                ) {
                    return None;
                }
                call.patterns.push(text(el.as_expression()?)?);
            }
        }
        e => call.patterns.push(text(e)?),
    }
    if let Some(opts) = args.next() {
        let Some(Expression::ObjectExpression(o)) = opts.as_expression() else {
            return None;
        };
        for p in &o.properties {
            let ObjectPropertyKind::ObjectProperty(p) = p else {
                return None;
            };
            let key = match &p.key {
                PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                PropertyKey::StringLiteral(s) => s.value.to_string(),
                _ => return None,
            };
            match (key.as_str(), &p.value) {
                ("eager", Expression::BooleanLiteral(b)) => call.eager = b.value,
                ("exhaustive", Expression::BooleanLiteral(b)) => call.exhaustive = b.value,
                ("import", v) => call.import = Some(text(v)?),
                _ => return None,
            }
        }
    }
    if args.next().is_some() || call.patterns.is_empty() {
        return None;
    }
    Some(call)
}

/// The call's matches as (key, import specifier from the module), sorted by key; `None` when a
/// pattern is not relative or root-absolute (an alias: left to throw).
fn matches(call: &Call, file: &Path, dir: &Path, root: &Path) -> Option<Vec<(String, String)>> {
    let anchored = |p: &str| p.starts_with("./") || p.starts_with("../") || p.starts_with('/');
    let (neg, pos): (Vec<&String>, Vec<&String>) =
        call.patterns.iter().partition(|p| p.starts_with('!'));
    let neg: Vec<&str> = neg.iter().map(|p| &p[1..]).collect();
    if !pos.iter().all(|p| anchored(p)) || !neg.iter().all(|p| anchored(p)) {
        return None;
    }
    let me = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut found: Vec<(String, String)> = Vec::new();
    for pat in pos {
        let segs: Vec<&str> = pat.split('/').collect();
        let fixed = segs[..segs.len() - 1]
            .iter()
            .take_while(|s| !s.contains(['*', '?', '[', ']', '{', '}']))
            .count();
        let prefix = segs[..fixed].join("/");
        let base = if pat.starts_with('/') {
            root.join(prefix.trim_start_matches('/'))
        } else {
            dir.join(&prefix)
        };
        let rest = segs[fixed..].join("/");
        let dots = call.exhaustive || rest.starts_with('.') || rest.contains("/.");
        let mut files = Vec::new();
        walk_dir(&base, "", dots, call.exhaustive, &mut files);
        for (rel, abs) in files {
            let key = format!("{prefix}/{rel}");
            if !crate::coverage::glob_matches(pat, &key)
                || neg.iter().any(|n| crate::coverage::glob_matches(n, &key))
                || abs.canonicalize().unwrap_or_else(|_| abs.clone()) == me
                || found.iter().any(|(k, _)| *k == key)
            {
                continue;
            }
            let spec = if pat.starts_with('/') {
                relative_spec(dir, &abs)
            } else {
                key.clone()
            };
            found.push((key, spec));
        }
    }
    found.sort();
    Some(found)
}

/// Every file under `dir` as (path relative to the walk's start, absolute path); dot entries
/// only with `dots`, `node_modules` only when `exhaustive`.
fn walk_dir(dir: &Path, rel: &str, dots: bool, exhaustive: bool, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if (!dots && name.starts_with('.')) || (!exhaustive && name == "node_modules") {
            continue;
        }
        let path = e.path();
        let r = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if path.is_dir() {
            walk_dir(&path, &r, dots, exhaustive, out);
        } else {
            out.push((r, path));
        }
    }
}

/// `target` as an import specifier from `dir` (`./x`, `../x`).
fn relative_spec(dir: &Path, target: &Path) -> String {
    let norm = |p: &Path| -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                Component::Normal(s) => v.push(s.to_string_lossy().into_owned()),
                Component::ParentDir => {
                    v.pop();
                }
                _ => {}
            }
        }
        v
    };
    let (from, to) = (norm(dir), norm(target));
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    if parts.is_empty() {
        parts.push(".".to_string());
    }
    parts.extend(to[common..].iter().cloned());
    parts.join("/")
}

#[cfg(test)]
#[path = "import_glob_tests.rs"]
mod tests;
