//! Explicit pnpm hoist ancestry fallback, without guessing store package versions.
//!
//! Normal resolution always runs first. An opted-in miss can use an ancestor's
//! `.pnpm/node_modules/<package>` link. Unlinked store versions are never scanned.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn enabled() -> bool {
    enabled_value(std::env::var_os("TURBO_PNPM_ANCESTOR").as_deref())
}
fn enabled_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}
fn invalid_bare(spec: &str) -> bool {
    spec.is_empty()
        || spec.starts_with('.')
        || spec.starts_with('/')
        || spec.starts_with('#')
        || spec.contains('\\')
        || spec.contains(':')
}
fn package_name(spec: &str) -> Option<&str> {
    if invalid_bare(spec) {
        return None;
    }
    let first = spec.split('/').next()?;
    if !first.starts_with('@') {
        return Some(first);
    }
    scoped_name(spec)
}
fn scoped_name(spec: &str) -> Option<&str> {
    let (scope, rest) = spec.split_once('/')?;
    let name = rest.split('/').next()?;
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(&spec[..scope.len() + 1 + name.len()])
}
fn installed_normally(from: &Path, package: &str) -> bool {
    from.ancestors()
        .any(|ancestor| ancestor.join("node_modules").join(package).is_dir())
}

/// Try the nearest explicit hoist link once. A blocked/unusable nearest package
/// must not silently select another ancestor/version or bypass package exports.
pub fn resolve(
    spec: &str,
    from: &Path,
    opted_in: bool,
    mut lookup: impl FnMut(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if !opted_in {
        return None;
    }
    let package = package_name(spec)?;
    if installed_normally(from, package) {
        return None;
    }
    for ancestor in from.ancestors() {
        let store = ancestor.join("node_modules/.pnpm");
        if store.join("node_modules").join(package).is_dir() {
            return lookup(&store);
        }
    }
    None
}

#[cfg(test)]
#[path = "pnpm_ancestor_tests.rs"]
mod tests;
