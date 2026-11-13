//! Canonical paths in the form Node and the module registry use (turbo-test#230).
//!
//! On Windows `std::fs::canonicalize` returns an extended-length ("verbatim") path:
//! `\\?\C:\proj\a.ts`, `\\?\UNC\server\share\a.ts`. Node's `fs.realpathSync` returns
//! `C:\proj\a.ts` and `\\server\share\a.ts`, and that is the spelling `node:path`, `URL`,
//! `__dirname` and the module registry see everywhere else; a verbatim path reaching them gives a
//! second registry entry for the same module and a `__dirname` `node:path` cannot walk. Every
//! canonical path the runner hands on goes through [`canonicalize`], which drops the prefix.
//! On other platforms the prefix never occurs, so the result is `std::fs::canonicalize`'s.

use std::path::{Path, PathBuf};

/// `std::fs::canonicalize` without Windows' verbatim prefix (see the module docs).
pub fn canonicalize(p: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(p).map(strip_verbatim)
}

/// On Windows, `p` with a verbatim drive (`\\?\C:\…` → `C:\…`) or UNC (`\\?\UNC\srv\share\…` →
/// `\\srv\share\…`) prefix removed. On Unix the input is preserved. Any other path, a
/// verbatim device path (`\\?\Volume{…}`,
/// `\\?\GLOBALROOT\…`) included, which has no plain spelling, comes back unchanged.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        strip_windows_verbatim(p)
    }
    #[cfg(not(windows))]
    {
        p
    }
}

#[cfg(any(windows, test))]
fn strip_windows_verbatim(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else {
        return p;
    };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if let Some(out) = strip_verbatim_drive(rest) {
            return out;
        }
    }
    p
}

/// `rest` (after `\\?\`) as a plain drive path if it opens with `X:` followed by end or `\`.
#[cfg(any(windows, test))]
fn strip_verbatim_drive(rest: &str) -> Option<PathBuf> {
    let b = rest.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if drive && (b.len() == 2 || b[2] == b'\\') {
        return Some(PathBuf::from(rest));
    }
    None
}

#[cfg(test)]
#[path = "canon_tests.rs"]
mod tests;
