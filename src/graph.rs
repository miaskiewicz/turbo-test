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
    let canon = |p: &Path| crate::canon::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let changed: HashSet<PathBuf> = changed.iter().map(|p| canon(p)).collect();
    let mut memo = HashMap::new();
    let mut out = Vec::new();
    for t in tests {
        let tc = canon(t);
        if changed.contains(&tc) {
            out.push(t.clone());
            continue;
        }
        let deps = transitive_imports(&tc, &mut memo);
        if deps.iter().any(|d| changed.contains(d)) {
            out.push(t.clone());
        }
    }
    out
}

#[cfg(all(test, unix))]
#[path = "graph_tests.rs"]
mod tests;
