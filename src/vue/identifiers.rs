//! Identifier and Component extraction helpers for Vue SFC compilation.

use std::path::Path;

/// Derives a clean component name from file path.
pub fn component_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("AnonymousComponent")
        .to_string()
}

/// Extract declared variable and function identifiers in setup to return them for the template.
pub fn extract_setup_identifiers(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_destructure = false;
    let mut destructure_buf = String::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if in_destructure {
            destructure_buf.push_str(trimmed);
            destructure_buf.push(' ');
            if trimmed.contains('}') || trimmed.contains(']') {
                in_destructure = false;
                parse_destructure(&destructure_buf, &mut names);
                destructure_buf.clear();
            }
            continue;
        }

        if trimmed.starts_with("const ") || trimmed.starts_with("let ") || trimmed.starts_with("var ") {
            let after_keyword = trimmed
                .strip_prefix("const ")
                .or_else(|| trimmed.strip_prefix("let "))
                .or_else(|| trimmed.strip_prefix("var "))
                .unwrap_or(trimmed);

            if after_keyword.starts_with('{') || after_keyword.starts_with('[') {
                if !after_keyword.contains('}') && !after_keyword.contains(']') {
                    in_destructure = true;
                    destructure_buf.push_str(after_keyword);
                    destructure_buf.push(' ');
                } else {
                    parse_destructure(after_keyword, &mut names);
                }
            } else {
                let id = after_keyword
                    .split(|c: char| c == ':' || c == '=' || c.is_whitespace())
                    .next()
                    .unwrap_or("");
                add_valid_ident(id, &mut names);
            }
        } else if trimmed.starts_with("function ") {
            let after_fn = trimmed.strip_prefix("function ").unwrap_or("");
            let id = after_fn
                .split(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or("");
            add_valid_ident(id, &mut names);
        }
    }
    names
}

pub fn parse_destructure(s: &str, names: &mut Vec<String>) {
    let start = s.find(['{', '[']);
    let end = s.rfind(['}', ']']);
    if let (Some(st), Some(en)) = (start, end) {
        if st < en {
            let inner = &s[st + 1..en];
            for part in inner.split(',') {
                let p = part.trim();
                let actual = if let Some((_, right)) = p.split_once(':') {
                    right.trim()
                } else if let Some((left, _)) = p.split_once('=') {
                    left.trim()
                } else {
                    p
                };
                let id = actual
                    .split(|c: char| c == '=' || c.is_whitespace())
                    .next()
                    .unwrap_or("");
                add_valid_ident(id, names);
            }
        }
    }
}

pub fn add_valid_ident(id: &str, names: &mut Vec<String>) {
    let id = id.trim();
    if !id.is_empty()
        && (id.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$'))
        && id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && !matches!(
            id,
            "if" | "else"
                | "return"
                | "const"
                | "let"
                | "var"
                | "function"
                | "import"
                | "export"
                | "type"
                | "interface"
                | "true"
                | "false"
                | "null"
                | "undefined"
        )
        && !names.iter().any(|n| n == id)
    {
        names.push(id.to_string());
    }
}

/// Extract imported component names (PascalCase) for component option registration.
pub fn extract_component_names(imports: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    let mut combined = String::new();
    for line in imports {
        let t = line.trim();
        if t.starts_with("import type ") || t.starts_with("import type{") {
            continue;
        }
        combined.push_str(t);
        combined.push(' ');
        if t.contains("from ") || t.ends_with(';') {
            parse_import_for_components(&combined, &mut names);
            combined.clear();
        }
    }
    if !combined.is_empty() {
        parse_import_for_components(&combined, &mut names);
    }
    names.sort();
    names.dedup();
    names
}

pub fn parse_import_for_components(s: &str, names: &mut Vec<String>) {
    let s = s.trim();
    if !s.starts_with("import ") || s.starts_with("import type ") {
        return;
    }
    let after = s.strip_prefix("import ").unwrap_or("").trim();
    let clause = if let Some((left, _)) = after.split_once(" from ") {
        left.trim()
    } else if let Some((left, _)) = after.split_once(" from\n") {
        left.trim()
    } else {
        return;
    };

    for part in clause.split(',') {
        let p = part.trim();
        if p.contains('{') || p.contains('}') {
            let inner = p.trim_matches(|c| c == '{' || c == '}').trim();
            for item in inner.split(',') {
                let item = item.trim();
                let actual = if let Some((_, as_name)) = item.split_once(" as ") {
                    as_name.trim()
                } else {
                    item
                };
                let id = actual.split_whitespace().next().unwrap_or("");
                if let Some(first) = id.chars().next() {
                    if first.is_ascii_uppercase() {
                        names.push(id.to_string());
                    }
                }
            }
        } else if let Some((_, as_name)) = p.split_once(" as ") {
            let id = as_name.trim();
            if let Some(first) = id.chars().next() {
                if first.is_ascii_uppercase() {
                    names.push(id.to_string());
                }
            }
        } else {
            let id = p.split_whitespace().next().unwrap_or("");
            if let Some(first) = id.chars().next() {
                if first.is_ascii_uppercase() {
                    names.push(id.to_string());
                }
            }
        }
    }
}

