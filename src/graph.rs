//! M5 — module dependency graph + affected-test detection (watch mode).
//!
//! On a file change, compute the minimal set of test files that transitively import it,
//! and run only those. Correctness rule (spec §M5): the computed set MUST be a SUPERSET of
//! the truly-affected tests — never under-select (a missed affected test is a correctness
//! bug). Over-selection is merely wasted work and is allowed/measured.
//!
//! Pure static analysis (no V8): extract import specifiers, resolve them (reusing the same
//! resolver as the runtime loader), build each test's transitive first-party import set.
//! node_modules deps are treated as leaves (a dependency change there triggers a full run).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::runner::resolve_spec;

/// Generously extract module specifiers from source: any quoted string following
/// `from` / `import` / `require` (covers static import/export-from, bare import,
/// dynamic import(), and require()). Generous = superset-safe.
pub fn extract_specifiers(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for kw in ["from", "import", "require"] {
        collect_kw_specifiers(src, kw, &mut out);
    }
    out
}

fn collect_kw_specifiers(src: &str, kw: &str, out: &mut Vec<String>) {
    let bytes = src.as_bytes();
    let mut i = 0;
    while let Some(p) = src[i..].find(kw) {
        let at = i + p;
        let after = at + kw.len();
        i = after;
        // word boundary before keyword
        if at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        if let Some(spec) = quoted_after_kw(&src[after..]) {
            out.push(spec);
        }
    }
}

fn quoted_after_kw(rest: &str) -> Option<String> {
    let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
    let q = trimmed.chars().next()?;
    if q != '\'' && q != '"' && q != '`' {
        return None;
    }
    let end = trimmed[1..].find(q)?;
    Some(trimmed[1..1 + end].to_string())
}

fn is_node_modules(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str() == "node_modules")
}

/// Transitive first-party import closure of a file (node_modules deps included as leaves,
/// not descended). Memoized; cycle-safe.
pub fn transitive_imports(
    file: &Path,
    memo: &mut HashMap<PathBuf, HashSet<PathBuf>>,
) -> HashSet<PathBuf> {
    if let Some(s) = memo.get(file) {
        return s.clone();
    }
    memo.insert(file.to_path_buf(), HashSet::new()); // cycle guard
    let mut set = HashSet::new();
    if let Ok(src) = std::fs::read_to_string(file) {
        let dir = file.parent().unwrap_or(Path::new("."));
        collect_deps_into(&src, dir, &mut set, memo);
    }
    memo.insert(file.to_path_buf(), set.clone());
    set
}

fn collect_deps_into(
    src: &str,
    dir: &Path,
    set: &mut HashSet<PathBuf>,
    memo: &mut HashMap<PathBuf, HashSet<PathBuf>>,
) {
    for spec in extract_specifiers(src) {
        let Some(dep) = resolve_spec(&spec, dir) else {
            continue;
        };
        if set.insert(dep.clone()) && !is_node_modules(&dep) {
            for t in transitive_imports(&dep, memo) {
                set.insert(t);
            }
        }
    }
}

/// Given changed files, return the subset of `tests` transitively affected.
/// Guaranteed superset of the truly-affected set.
pub fn affected_tests(changed: &[PathBuf], tests: &[PathBuf]) -> Vec<PathBuf> {
    affected_tests_with_aliases(changed, tests, &[])
}

/// Resolve each test independently: sharing a partial closure across a cycle can miss edges.
pub fn affected_tests_with_aliases(
    changed: &[PathBuf],
    tests: &[PathBuf],
    aliases: &[(String, String)],
) -> Vec<PathBuf> {
    let canon = |p: &Path| crate::canon::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let changed: HashSet<PathBuf> = changed.iter().map(|p| canon(p)).collect();
    tests
        .iter()
        .filter(|test| test_is_affected(test, &changed, aliases))
        .cloned()
        .collect()
}

fn canonical(path: &Path) -> PathBuf {
    crate::canon::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn test_is_affected(test: &Path, changed: &HashSet<PathBuf>, aliases: &[(String, String)]) -> bool {
    crate::runner::set_custom_resolve_options(test);
    let mut pending = crate::runner::dependency_setup_files(test);
    pending.push(canonical(test));
    let mut seen = HashSet::new();
    while let Some(file) = pending.pop() {
        if visit_test_file(&canonical(&file), changed, aliases, &mut pending, &mut seen) {
            return true;
        }
    }
    false
}

fn visit_test_file(
    file: &Path,
    changed: &HashSet<PathBuf>,
    aliases: &[(String, String)],
    pending: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) -> bool {
    if !seen.insert(file.to_path_buf()) {
        return false;
    }
    if changed.contains(file) || snapshot_changed(file, changed) {
        return true;
    }
    if is_node_modules(file) {
        return false;
    }
    let Ok(src) = std::fs::read_to_string(file) else {
        return false;
    };
    if !changed.is_empty() && has_computed_import(&src) {
        return true;
    }
    append_resolved_imports(file, &src, aliases, pending);
    false
}

fn append_resolved_imports(
    file: &Path,
    src: &str,
    aliases: &[(String, String)],
    pending: &mut Vec<PathBuf>,
) {
    let dir = file.parent().unwrap_or(Path::new("."));
    for spec in extract_specifiers(src) {
        let mapped = aliases
            .iter()
            .find_map(|(from, to)| map_alias(&spec, from, to));
        if let Some(dep) = resolve_spec(mapped.as_deref().unwrap_or(&spec), dir) {
            pending.push(dep);
        }
    }
}

fn map_alias(spec: &str, from: &str, to: &str) -> Option<String> {
    if spec == from {
        return Some(to.to_string());
    }
    spec.strip_prefix(from)
        .filter(|rest| rest.starts_with('/'))
        .map(|rest| format!("{to}{rest}"))
}

fn snapshot_changed(file: &Path, changed: &HashSet<PathBuf>) -> bool {
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else {
        return false;
    };
    let mut snapshot = name.to_os_string();
    snapshot.push(".snap");
    changed.contains(&parent.join("__snapshots__").join(snapshot))
}

fn has_computed_import(source: &str) -> bool {
    if source.contains("import.meta.glob") {
        return true;
    }
    ["import", "require"].iter().any(|keyword| {
        source.match_indices(keyword).any(|(index, _)| {
            let tail = source[index + keyword.len()..].trim_start();
            let Some(argument) = tail.strip_prefix('(') else {
                return false;
            };
            let argument = argument.trim_start();
            match argument.chars().next() {
                Some('\'' | '"') => false,
                Some('`') => argument[1..]
                    .split('`')
                    .next()
                    .is_some_and(|s| s.contains("${")),
                _ => true,
            }
        })
    })
}

#[cfg(all(test, unix))]
#[path = "graph_tests.rs"]
mod tests;
