//! A failure as vitest's reporters print it (turbo-test#81): the error's name, message, the diff
//! of its expected / actual values (computed in runtime.js `__errorInfo` as @vitest/utils'
//! `processError` does), its stack frames and its cause, rendered the way `printErrorInner`
//! (`capturePrintError`, no code frame) writes them into a JUnit `<failure>`.

use std::path::{Path, PathBuf};

/// One stack frame: the function name V8 gives (empty for an anonymous one) and the position,
/// 1-based, in the original source when the file's transform map is known.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub method: String,
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
}

/// An error a test (or a hook, or the file's load) failed with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ErrorInfo {
    pub name: String,
    pub message: String,
    /// `- Expected` / `+ Received` diff (or `Expected:` / `Received:` lines); empty for none.
    pub diff: String,
    pub frames: Vec<Frame>,
    pub cause: Option<Box<ErrorInfo>>,
}

/// The frames of a V8 `stack` (`    at fn (/abs/file.ts:3:9)` / `    at /abs/file.ts:3:9`) that
/// point into a file: frames of scripts without a path (`<anonymous>`, the runtime's own) are
/// left out, as vitest's stack parser drops them. `async ` and a `file://` prefix are dropped,
/// and so is an `Object.` receiver: the module runner calls an imported function on its
/// module's exports object (`import_h.helper()`), where vitest's module objects have no
/// constructor name, so V8 names vitest's frame `helper`.
pub fn parse_stack(stack: &str) -> Vec<Frame> {
    stack
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("at "))
        .filter_map(|rest| {
            let (method, loc) = match rest.strip_suffix(')').and_then(|r| r.rsplit_once(" (")) {
                Some((m, loc)) => (m, loc),
                None => ("", rest),
            };
            let loc = loc.strip_prefix("file://").unwrap_or(loc);
            let (file_line, col) = loc.rsplit_once(':')?;
            let (file, line) = file_line.rsplit_once(':')?;
            if !file.starts_with('/') {
                return None;
            }
            Some(Frame {
                method: {
                    let m = method.strip_prefix("async ").unwrap_or(method);
                    m.strip_prefix("Object.").unwrap_or(m).to_string()
                },
                file: PathBuf::from(file),
                line: line.parse().ok()?,
                col: col.parse().ok()?,
            })
        })
        .collect()
}

/// `capturePrintError(error).output.trim()`: `Name: message`, the diff between blank lines, one
/// ` ❯ [method ]path:line:col` per frame outside node_modules (paths relative to `root`) and a
/// blank line after them, then the cause as `Caused by: Name: message` and its frames.
pub fn print_error(e: &ErrorInfo, root: &Path) -> String {
    let mut out = String::new();
    write_error(&mut out, e, root, "");
    out.trim().to_string()
}

fn write_error(out: &mut String, e: &ErrorInfo, root: &Path, prefix: &str) {
    out.push_str(&format!("{prefix}{}: {}\n", e.name, e.message));
    if !e.diff.is_empty() {
        out.push_str(&format!("\n{}\n\n", e.diff));
    }
    let frames: Vec<&Frame> = e
        .frames
        .iter()
        .filter(|f| !f.file.components().any(|c| c.as_os_str() == "node_modules"))
        .collect();
    for f in &frames {
        let path = f.file.strip_prefix(root).unwrap_or(&f.file);
        let at = format!("{}:{}:{}", path.display(), f.line, f.col);
        match f.method.as_str() {
            "" => out.push_str(&format!(" ❯ {at}\n")),
            m => out.push_str(&format!(" ❯ {m} {at}\n")),
        }
    }
    if !frames.is_empty() {
        out.push('\n');
    }
    if let Some(cause) = &e.cause {
        write_error(out, cause, root, "Caused by: ");
    }
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
