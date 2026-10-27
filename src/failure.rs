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

/// An error a test (or a hook, or the file's load) failed with: @vitest/utils' serialized error
/// (a thrown primitive is `{ message }`, turbo-test#97).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ErrorInfo {
    /// Its name, `Unknown Error` for one without.
    pub name: String,
    pub message: String,
    /// It has no `name` / `message` (junit leaves the `type` / `message` attribute out; the
    /// verbose reporter prints the message as `undefined`).
    pub no_name: bool,
    pub no_message: bool,
    /// What vitest prints instead of `Name: message` for a value without a message: the
    /// serialized error as Node's console shows it (`{ a: 1, stacks: [] }`).
    pub inspected: Option<String>,
    /// vitest's `Serialized Error`: the error's own properties (loupe), `None` for none.
    pub properties: Option<String>,
    /// `- Expected` / `+ Received` diff (or `Expected:` / `Received:` lines); empty for none.
    pub diff: String,
    pub frames: Vec<Frame>,
    pub cause: Option<Box<ErrorInfo>>,
    /// The module whose compile the error is (turbo-test#99). Not printed: it tells apart two
    /// otherwise identical compile errors as vitest's stacks do (the human reporters print one
    /// module's error once, under every file whose import of it failed).
    pub source: Option<std::path::PathBuf>,
    /// What vite's plugin container added to a module's parse / transform error (vitest's
    /// `__vitest_rollup_error__`, printed under the message; turbo-test#113).
    pub rollup: Option<RollupDetails>,
}

/// vite's details of a load error: the plugin that threw, the module's id, its `line:column`
/// and vite's code frame (each empty when vite gave none).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RollupDetails {
    pub plugin: String,
    pub id: String,
    pub loc: String,
    pub frame: String,
}

/// The frames of a V8 `stack` (`    at fn (/abs/file.ts:3:9)` / `    at /abs/file.ts:3:9`) that
/// point into a file: frames of scripts without a path (`<anonymous>`, the runtime's own) are
/// left out, as vitest's stack parser drops them, and so are the frames its
/// `stackIgnorePatterns` match ([`ignored`]). `async ` and a `file://` prefix are dropped,
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
                // an anonymous frame: `at <location>`, or `at async <location>` when it is
                // suspended at an await (turbo-test#169)
                None => ("", rest.strip_prefix("async ").unwrap_or(rest)),
            };
            let loc = loc.strip_prefix("file://").unwrap_or(loc);
            let (file_line, col) = loc.rsplit_once(':')?;
            let (file, line) = file_line.rsplit_once(':')?;
            if !file.starts_with('/') || ignored(file) {
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

/// @vitest/utils' default `stackIgnorePatterns`: the frames vitest's stack parser leaves out
/// (its own packages, chai, tinyspy, node internals). Other node_modules frames are kept
/// (turbo-test#173).
fn ignored(file: &str) -> bool {
    const PARTS: [&str; 17] = [
        "node:internal",
        "/vitest/dist/",
        "/vitest/src/",
        "/node_modules/chai/",
        "/node_modules/tinyspy/",
        "/vite/dist/node/module-runner",
        "/rolldown-vite/dist/node/module-runner",
        "/deps/chunk-",
        "/deps/@vitest",
        "/deps/loupe",
        "/deps/chai",
        "/browser-playwright/dist/locators.js",
        "/browser-webdriverio/dist/locators.js",
        "/browser-preview/dist/locators.js",
        "__vitest_test__",
        "__vitest_browser__",
        "/@id/__x00__vitest/browser",
    ];
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // `<prefix>\w+<suffix>`
    let word_then = |prefix: &str, suffix: &str| {
        file.match_indices(prefix).any(|(i, _)| {
            let rest = &file[i + prefix.len()..];
            let n = rest.find(|c| !word(c)).unwrap_or(rest.len());
            n > 0 && rest[n..].starts_with(suffix)
        })
    };
    PARTS.iter().any(|p| file.contains(p))
        || file.contains("/deps/vitest_")
        || word_then("/packages/", "/dist/")
        || word_then("/@vitest/", "/dist/")
        || word_then("node:", "")
}

/// `capturePrintError(error).output.trim()` (junit's `<failure>` body): vitest's printErrorInner
/// without a code frame, with the error's properties ([`crate::human::print_error`]).
pub fn print_error(e: &ErrorInfo, root: &Path) -> String {
    crate::human::print_error(e, root, false, true, "", &crate::colors::Colors::default())
        .trim()
        .to_string()
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
