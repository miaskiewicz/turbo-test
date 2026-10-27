//! vite's `import.meta.glob` transform (turbo-test#130, #145), as vitest 4.1.12 runs it in tests:
//! a call by the full name with literal arguments is replaced, before any other transform, by an
//! object of the matching modules — eager (`{ eager: true }`: the module namespaces, imported
//! statically ahead of the module's own imports) or lazy (`() => import(...)`), each value the
//! module or its `import` export. Patterns are string literals (or an array of them; `!` negates)
//! relative to the module (`./`, `../`), the project root (`/`) or a `resolve.alias`; `base`
//! moves relative patterns and the keys to another directory; `query` (string or object) and the
//! deprecated `as` add a `?query` to each import. Keys are as vite writes them, sorted: relative
//! to `base`, else to the module for a relative pattern, else root-relative. As vite: the module
//! itself, dot files and `node_modules` are skipped unless `exhaustive: true`. A pattern it
//! cannot anchor fails the module with vite's "Invalid glob" error (thrown on its first line);
//! a call it cannot replace (another option, a non-literal argument) is left alone and throws
//! vitest's error at run time (runtime.js).
//!
//! The replacement keeps the call's line count and the eager imports go on the first line, so
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
    /// the `?query` added to each import (`query`, or `as: X` -> `?X`)
    query: String,
    base: Option<String>,
}

/// `src` (the module at `file`) with every replaceable `import.meta.glob` call expanded against
/// the file system, `root` the base of `/` patterns, `aliases` the config's `resolve.alias`
/// (find, replacement); `None` when there is none. The eager imports go first, on the first
/// line (after a hashbang), so they evaluate before the module's own imports, as vite's do.
pub fn expand(file: &Path, src: &str, root: &Path, aliases: &[(String, String)]) -> Option<String> {
    if !src.contains("glob") || !src.contains("import.meta") {
        return None;
    }
    let calls = glob_calls(file, src);
    if calls.is_empty() {
        return None;
    }
    let dir = file.parent()?;
    // vite fails the module at transform on a pattern it cannot anchor (turbo-test#149): the
    // module throws vite's error when it is evaluated, before any of its code runs
    let invalid = calls.iter().flat_map(|c| &c.patterns).find_map(|p| {
        let p = p.strip_prefix('!').unwrap_or(p);
        anchor(p, dir, root, aliases).is_none().then(|| {
            format!("Invalid glob: \"{p}\" (resolved: \"{p}\"). It must start with '/' or './'")
        })
    });
    if let Some(message) = invalid {
        let at = if src.starts_with("#!") {
            src.find('\n').map_or(src.len(), |n| n + 1)
        } else {
            0
        };
        let mut out = src.to_string();
        let message = serde_json::to_string(&message).ok()?;
        out.insert_str(at, &format!("throw new Error({message});"));
        return Some(out);
    }
    let mut out = src.to_string();
    let mut imports: Vec<String> = Vec::new();
    let mut changed = false;
    for (i, call) in calls.iter().enumerate().rev() {
        let Some(found) = matches(call, file, dir, root, aliases) else {
            continue;
        };
        let mut entries = Vec::new();
        let mut eager = String::new();
        for (j, (key, spec)) in found.iter().enumerate() {
            let spec = serde_json::to_string(&format!("{spec}{}", call.query)).ok()?;
            let pick = call
                .import
                .as_ref()
                .map(|n| format!("[{}]", serde_json::to_string(n).unwrap_or_default()))
                .unwrap_or_default();
            let value = if call.eager {
                let local = format!("__tt_glob_{i}_{j}");
                eager.push_str(&format!("import * as {local} from {spec};"));
                format!("{local}{pick}")
            } else if pick.is_empty() {
                format!("() => import({spec})")
            } else {
                format!("() => import({spec}).then((m) => m{pick})")
            };
            entries.push(format!("{}: {value}", serde_json::to_string(key).ok()?));
        }
        imports.push(eager);
        let (start, end) = (call.span.0 as usize, call.span.1 as usize);
        let lines = "\n".repeat(src[start..end].matches('\n').count());
        let replacement = format!("({{{}}}{lines})", entries.join(", "));
        out.replace_range(start..end, &replacement);
        changed = true;
    }
    if !changed {
        return None;
    }
    imports.reverse();
    let imports = imports.concat();
    let at = if out.starts_with("#!") {
        out.find('\n').map_or(out.len(), |n| n + 1)
    } else {
        0
    };
    out.insert_str(at, &imports);
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
        query: String::new(),
        base: None,
    };
    let mut r#as: Option<String> = None;
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
                ("base", v) => call.base = Some(text(v)?),
                // deprecated `as: 'raw'` = `query: '?raw', import: 'default'`
                ("as", v) => r#as = Some(text(v)?),
                ("query", Expression::ObjectExpression(q)) => {
                    let mut pairs = Vec::new();
                    for p in &q.properties {
                        let ObjectPropertyKind::ObjectProperty(p) = p else {
                            return None;
                        };
                        let k = match &p.key {
                            PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                            PropertyKey::StringLiteral(s) => s.value.to_string(),
                            _ => return None,
                        };
                        let v = match &p.value {
                            Expression::BooleanLiteral(b) => b.value.to_string(),
                            Expression::NumericLiteral(n) => n.value.to_string(),
                            v => text(v)?,
                        };
                        pairs.push(format!("{}={}", form_encode(&k), form_encode(&v)));
                    }
                    if !pairs.is_empty() {
                        call.query = format!("?{}", pairs.join("&"));
                    }
                }
                ("query", v) => {
                    let q = text(v)?;
                    call.query = match q.as_str() {
                        "" => q,
                        _ if q.starts_with('?') => q,
                        _ => format!("?{q}"),
                    };
                }
                _ => return None,
            }
        }
    }
    if let Some(a) = r#as {
        if call.query.is_empty() {
            call.query = format!("?{a}");
        }
        if call.import.is_none() {
            call.import = Some("default".to_string());
        }
    }
    if args.next().is_some() || call.patterns.is_empty() {
        return None;
    }
    Some(call)
}

/// The call's matches as (key, import specifier from the module), sorted by key; `None` when a
/// pattern is neither relative, root-absolute nor an alias (vite: an invalid glob). Keys are as
/// vite writes them: relative to `base` when there is one, else relative to the module for a
/// relative pattern, else root-relative (`/…`).
fn matches(
    call: &Call,
    file: &Path,
    dir: &Path,
    root: &Path,
    aliases: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    // the base directory: the whole path (its last segment is no glob)
    let base = match &call.base {
        Some(b) => {
            let (d, rest, _) = anchor(b, dir, root, aliases)?;
            Some(normal(&d.join(rest)))
        }
        None => None,
    };
    let from = base.as_deref().unwrap_or(dir);
    let mut pos = Vec::new();
    let mut neg = Vec::new();
    for p in &call.patterns {
        let (p, list) = match p.strip_prefix('!') {
            Some(p) => (p, &mut neg),
            None => (p.as_str(), &mut pos),
        };
        list.push(anchor(p, from, root, aliases)?);
    }
    let me = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut found: Vec<(String, String)> = Vec::new();
    for (start, rest, relative) in &pos {
        let dots = call.exhaustive || rest.starts_with('.') || rest.contains("/.");
        let mut files = Vec::new();
        walk_dir(start, "", dots, call.exhaustive, &mut files);
        for (rel, abs) in files {
            let excluded = neg.iter().any(|(nd, nrest, _)| {
                abs.strip_prefix(nd)
                    .is_ok_and(|r| crate::coverage::glob_matches(nrest, &r.to_string_lossy()))
            });
            if !crate::coverage::glob_matches(rest, &rel)
                || excluded
                || abs.canonicalize().unwrap_or_else(|_| abs.clone()) == me
            {
                continue;
            }
            let key = match &base {
                Some(b) => relative_spec(b, &abs),
                None if *relative => relative_spec(dir, &abs),
                None => match normal(&abs).strip_prefix(normal(root)) {
                    Ok(r) => format!("/{}", r.to_string_lossy()),
                    Err(_) => abs.to_string_lossy().into_owned(),
                },
            };
            if found.iter().any(|(k, _)| *k == key) {
                continue;
            }
            found.push((key, relative_spec(dir, &abs)));
        }
    }
    found.sort();
    Some(found)
}

/// A pattern as (the directory its fixed leading segments name, the glob below it, whether it
/// is relative to `from`): `./` / `../` from `from`, `/` from `root`, an alias from its
/// replacement; `None` for anything else.
fn anchor(
    pattern: &str,
    from: &Path,
    root: &Path,
    aliases: &[(String, String)],
) -> Option<(PathBuf, String, bool)> {
    let alias = aliases.iter().find_map(|(find, to)| {
        let rest = pattern.strip_prefix(find.as_str())?;
        (rest.is_empty() || rest.starts_with('/')).then(|| {
            let to = Path::new(to);
            let to = if to.is_absolute() {
                to.to_path_buf()
            } else {
                root.join(to)
            };
            format!("{}{rest}", to.to_string_lossy())
        })
    });
    let (start, rest, relative) = match alias {
        Some(abs) => (
            PathBuf::from("/"),
            abs.trim_start_matches('/').to_string(),
            false,
        ),
        None if pattern.starts_with("./") || pattern.starts_with("../") => {
            (from.to_path_buf(), pattern.to_string(), true)
        }
        None if pattern.starts_with('/') => (
            root.to_path_buf(),
            pattern.trim_start_matches('/').to_string(),
            false,
        ),
        None => return None,
    };
    let segs: Vec<&str> = rest.split('/').collect();
    let fixed = segs[..segs.len() - 1]
        .iter()
        .take_while(|s| !s.contains(['*', '?', '[', ']', '{', '}']))
        .count();
    let dir = normal(&start.join(segs[..fixed].join("/")));
    Some((dir, segs[fixed..].join("/"), relative))
}

/// `p` with `.` and `..` folded away (lexically).
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `application/x-www-form-urlencoded` (URLSearchParams) encoding of a query key or value.
fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
