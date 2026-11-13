//! Explicit pnpm hoist ancestry fallback, without guessing store package versions.
//!
//! Normal resolution always runs first. An opted-in miss can use an ancestor's
//! `.pnpm/node_modules/<package>` link. Unlinked store versions are never scanned.
use std::cell::Cell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

thread_local! {
    static RUNTIME_FLAG: Cell<Option<bool>> = const { Cell::new(None) };
}

/// The current JS context's opt-in, scoped to one resolver boundary on its worker.
/// A missing runtime flag is false, not a fallback to another context or host mutation.
pub(crate) struct RuntimeFlagGuard {
    previous: Option<bool>,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for RuntimeFlagGuard {
    fn drop(&mut self) {
        RUNTIME_FLAG.with(|flag| flag.set(self.previous));
    }
}
pub(crate) fn runtime_flag(value: Option<&str>) -> RuntimeFlagGuard {
    let previous = RUNTIME_FLAG.with(|flag| flag.replace(Some(value == Some("1"))));
    RuntimeFlagGuard {
        previous,
        _same_thread: std::marker::PhantomData,
    }
}
pub fn enabled() -> bool {
    RUNTIME_FLAG
        .with(|flag| flag.get())
        .unwrap_or_else(|| enabled_value(std::env::var_os("TURBO_PNPM_ANCESTOR").as_deref()))
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
