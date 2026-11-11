//! CLI launcher — the front-end work that used to live in `cli.js` (the npm Node launcher),
//! ported to Rust so the binary is self-contained (no Node process needed to launch a run).
//!
//! Responsibilities (all formerly in cli.js):
//!   - strip a leading `run`/`watch`/`dev` subcommand (vitest dispatch token);
//!   - consume launcher-only flags (`-c/--config`, `--root/--dir`, `--environment`,
//!     `--changed [since]`, `--isolate/--no-isolate`, `--globals/--no-globals`,
//!     `--passWithNoTests`) and turn them into env vars / discovery options;
//!   - default test-file discovery (vitest-style) honoring a vitest/vite config's
//!     test-level `include`/`exclude` globs;
//!   - inject the vitest `coverage` block's thresholds/include/exclude when `--coverage*` is on
//!     and the user didn't pass them explicitly (flags win);
//!   - `--changed [since]` git filter;
//!   - prune file args that no longer exist.
//!
//! `prepare()` returns the effective argument vector — the runner-side flags it did NOT consume,
//! followed by the resolved absolute test-file paths — which `cli::parse_args`' flag loop
//! then parses unchanged. It may `std::process::exit` directly for the terminal cases
//! (`--passWithNoTests`, no files found, `--changed` with nothing changed), exactly as cli.js did.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::coverage::glob_matches;

/// vitest's default `test.exclude` (what applies when a config does not set one).
const DEFAULT_EXCLUDE_GLOBS: [&str; 2] = ["**/node_modules/**", "**/.git/**"];

/// The config files vitest 4 resolves, in its order (`configFiles` in vitest's constants:
/// `vitest.config` then `vite.config`, each with `.ts .mts .cts .js .mjs .cjs`). Every config
/// lookup (discovery, setupFiles, isolate, the project kind) uses this list.
pub const VITEST_CONFIG_FILES: [&str; 12] = [
    "vitest.config.ts",
    "vitest.config.mts",
    "vitest.config.cts",
    "vitest.config.js",
    "vitest.config.mjs",
    "vitest.config.cjs",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.cts",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.cjs",
];

/// Runner flags that take a following value (so a file arg isn't mistaken for the value, and the
/// value isn't mistaken for a file). Mirrors the value-flag set cli.js forwarded with their arg.
/// `cli::parse_args` also accepts each `--` one in its inline `--flag=value` form.
pub(crate) const VALUE_FLAGS: [&str; 23] = [
    "-j",
    "--jobs",
    "--shard",
    "--reporter",
    "--reporters",
    "--outputFile",
    "--output-file",
    "-t",
    "--testNamePattern",
    "--testTimeout",
    "--hookTimeout",
    "--maxConcurrency",
    "--retry",
    "--bail",
    "--maxWorkers",
    "--minWorkers",
    "--coverage-dir",
    "--coverage-thresholds",
    "--coverage-threshold",
    "--coverage-reporter",
    "--coverage-reporters",
    "--coverage-include",
    "--coverage-exclude",
];

/// `name` is a test file by vitest's default include: `*.{test,spec}.?(c|m)[jt]s?(x)`.
fn is_test_file(name: &str) -> bool {
    // strip the final extension, require it to be one of the known ones, then require the
    // remaining stem to end with `.test` or `.spec`.
    let exts = [
        "js", "ts", "jsx", "tsx", "cjs", "cts", "mjs", "mts", "cjsx", "ctsx", "mjsx", "mtsx",
    ];
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if !exts.contains(&ext) {
        return false;
    }
    stem.ends_with(".test") || stem.ends_with(".spec")
}

/// Recursive discovery walk: collect the files whose name `keep` accepts, not descending into the
/// directories `prune` accepts (symbolic links are not followed).
fn walk(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    keep: &dyn Fn(&str) -> bool,
    prune: &dyn Fn(&Path) -> bool,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let full = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() && !prune(&full) => walk(&full, out, keep, prune),
            Ok(ft) if ft.is_file() && keep(&name) => out.push(full),
            _ => {}
        }
    }
}

/// Skip ASCII whitespace forward from `i`. Returns the new index.
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Find the array literal for `key` — `key` followed by optional ws, `:`, optional ws, `[ ... ]` —
/// and return the quoted string items inside it. Mirrors cli.js `arr(key)` (a loose string-scan,
/// not a real parse): `key` is matched as a substring, the FIRST such occurrence whose punctuation
/// lines up wins. Returns `None` when no such `key:[...]` exists.
fn scan_string_array(text: &str, key: &str) -> Option<Vec<String>> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let after = from + rel + key.len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'[' {
                // read to the closing ']' ( [^\]]* — no nested brackets, like the JS regex)
                if let Some(close_rel) = text[i + 1..].find(']') {
                    let inner = &text[i + 1..i + 1 + close_rel];
                    return Some(extract_quoted(inner));
                }
            }
        }
        from = after;
    }
    None
}

/// Pull every quoted (`'` `"` or `` ` ``) string out of `s`, in order. Mirrors cli.js
/// `m[1].match(/['"`]([^'"`]+)['"`]/g)`.
fn extract_quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' || c == b'"' || c == b'`' {
            if let Some(end_rel) = s[i + 1..].find(['\'', '"', '`']) {
                let val = &s[i + 1..i + 1 + end_rel];
                if !val.is_empty() {
                    out.push(val.to_string());
                }
                i = i + 1 + end_rel + 1;
                continue;
            } else {
                break;
            }
        }
        i += 1;
    }
    out
}

/// A discovered vitest config and its directory (the discovery/project root).
struct Config {
    dir: PathBuf,
    text: String,
}

/// Locate a vitest/vite config: with `forced` (a `-c/--config` path) use that exact file (its dir
/// is the root); otherwise walk up from `start_dir`. Mirrors cli.js `findConfig`.
fn find_config(start_dir: &Path, forced: Option<&Path>) -> Option<Config> {
    if let Some(f) = forced {
        let p = std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf());
        let text = std::fs::read_to_string(&p).ok()?;
        let dir = p.parent().unwrap_or(Path::new(".")).to_path_buf();
        return Some(Config { dir, text });
    }
    let mut dir = start_dir.to_path_buf();
    loop {
        match config_in_dir(&dir) {
            ConfigScan::Found(c) => return Some(c),
            ConfigScan::Unreadable => return None,
            ConfigScan::Missing => {}
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

enum ConfigScan {
    Found(Config),
    Unreadable,
    Missing,
}

/// Look for a vitest/vite config in `dir` only (no ancestor walk).
fn config_in_dir(dir: &Path) -> ConfigScan {
    for n in VITEST_CONFIG_FILES {
        let p = dir.join(n);
        if p.is_file() {
            return match std::fs::read_to_string(&p) {
                Ok(text) => ConfigScan::Found(Config {
                    dir: dir.to_path_buf(),
                    text,
                }),
                Err(_) => ConfigScan::Unreadable,
            };
        }
    }
    ConfigScan::Missing
}

/// Test-level include/exclude globs + their root, from a discovered config (`None` without a
/// config). Either list is `None` when the config does not set it: vitest's default applies
/// ([`DEFAULT_INCLUDE`] / [`DEFAULT_EXCLUDE`]). The FIRST `include`/`exclude` arrays in the file
/// are the test-level ones (they precede any `coverage.*` block in the config object).
struct Patterns {
    root: PathBuf,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

/// vitest's default `test.include` (what [`is_test_file`] matches) and `test.exclude` (what
/// [`DEFAULT_EXCLUDE_GLOBS`] lists), as its "No test files found" message prints them.
const DEFAULT_INCLUDE: &str = "**/*.{test,spec}.?(c|m)[jt]s?(x)";
const DEFAULT_EXCLUDE: &str = "**/node_modules/**, **/.git/**";

fn vitest_patterns(start_dir: &Path, forced: Option<&Path>) -> Option<Patterns> {
    let cfg = find_config(start_dir, forced)?;
    // Scan include/exclude in the TEST config only — truncate before the coverage block. Otherwise
    // a project with no `test.exclude` but a `coverage.exclude` whose first entry is the test glob
    // (e.g. `**/*.test.{ts,tsx}`, common in vitest coverage configs) has that picked up as the test
    // exclude → every test file is excluded → "no test files found". test.* precedes coverage.* in
    // the config object, so the prefix before `coverage:` holds the real test patterns.
    let scan_text = match find_coverage_block(&cfg.text) {
        Some(ci) => &cfg.text[..ci],
        None => cfg.text.as_str(),
    };
    let include = scan_string_array(scan_text, "include");
    let exclude = scan_string_array(scan_text, "exclude");
    Some(Patterns {
        root: cfg.dir,
        include,
        exclude,
    })
}

/// Whether config text sets `key: true` (`key\s*:\s*true`, the first `key:` wins).
fn config_true(text: &str, key: &str) -> bool {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let after = from + rel + key.len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            return text[skip_ws(b, i + 1)..].starts_with("true");
        }
        from = after;
    }
    false
}

/// `server.deps.inline` (or the older `deps.inline`) from config text: its package names, or `*`
/// for `inline: true` (turbo-test#216). RegExp entries are not read.
fn config_deps_inline(text: &str) -> Vec<String> {
    let Some(d) = text.find("deps") else {
        return vec![];
    };
    let deps = &text[d..];
    if config_true(deps, "inline") {
        return vec!["*".to_string()];
    }
    scan_string_array(deps, "inline").unwrap_or_default()
}

/// Read `test.environment` (node|jsdom|happy-dom) from config text. Mirrors cli.js
/// `configEnvironment`: `environment\s*:\s*['"`]([a-z-]+)['"`]`, lowercased.
fn config_environment(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("environment") {
        let after = from + rel + "environment".len();
        if let Some(val) = environment_value_at(b, text, after) {
            return Some(val);
        }
        from = after;
    }
    None
}

/// After a match of `environment`, try to parse the `: '<name>'` value.
fn environment_value_at(b: &[u8], text: &str, after: usize) -> Option<String> {
    let i = skip_ws(b, after);
    if i >= b.len() || b[i] != b':' {
        return None;
    }
    let i = skip_ws(b, i + 1);
    if i >= b.len() || !matches!(b[i], b'\'' | b'"' | b'`') {
        return None;
    }
    let val: String = text[i + 1..]
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
        .collect();
    (!val.is_empty()).then(|| val.to_ascii_lowercase())
}

/// The vitest `coverage` block's include/exclude globs + thresholds string. Mirrors cli.js
/// `vitestCoverage`. Returns `None` only when there's no config at all.
struct CoverageCfg {
    include: Vec<String>,
    exclude: Vec<String>,
    thresholds: Option<String>,
}

fn vitest_coverage(start_dir: &Path, forced: Option<&Path>) -> Option<CoverageCfg> {
    let cfg = find_config(start_dir, forced)?;
    let text = &cfg.text;
    // slice from the `coverage:` key so include/exclude/thresholds resolve to the coverage block,
    // not the test-level ones (test.* precedes coverage.* in the config object).
    let Some(ci) = find_coverage_block(text) else {
        return Some(CoverageCfg {
            include: vec![],
            exclude: vec![],
            thresholds: None,
        });
    };
    let slice = &text[ci..];
    let include = scan_string_array(slice, "include").unwrap_or_default();
    let exclude = scan_string_array(slice, "exclude").unwrap_or_default();
    // thresholds can be `coverage.thresholds: { lines: 90, ... }` or the flat legacy form
    // (numbers directly under coverage). Prefer the nested block when present.
    let thr_text = scan_braced(slice, "thresholds").unwrap_or_else(|| slice.to_string());
    let mut parts = Vec::new();
    for k in ["lines", "functions", "branches", "statements"] {
        if let Some(v) = scan_number(&thr_text, k) {
            parts.push(format!("{k}={v}"));
        }
    }
    let thresholds = if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    };
    Some(CoverageCfg {
        include,
        exclude,
        thresholds,
    })
}

/// Byte offset of the `coverage` key whose value is an object (`coverage\s*:\s*\{`).
fn find_coverage_block(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("coverage") {
        let at = from + rel;
        let after = at + "coverage".len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'{' {
                return Some(at);
            }
        }
        from = after;
    }
    None
}

/// Contents between the braces of `key\s*:\s*\{ ... \}` (first match, no nesting — like the JS
/// `thresholds\s*:\s*\{([^}]*)\}`).
fn scan_braced(text: &str, key: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let after = from + rel + key.len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'{' {
                if let Some(close_rel) = text[i + 1..].find('}') {
                    return Some(text[i + 1..i + 1 + close_rel].to_string());
                }
            }
        }
        from = after;
    }
    None
}

/// Numeric or `Infinity` value for `key` (the leading guard prevents matching `key` as the
/// tail of another identifier so `branches` doesn't match `subBranches`). vitest's withTimeout
/// treats `<= 0 || Infinity` as "no timeout" (turbo-test#273), so `test.hookTimeout: Infinity`
/// in a config must forward too.
fn scan_number(text: &str, key: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let at = from + rel;
        let after = at + key.len();
        if is_key_boundary_prev(b, at) {
            if let Some(v) = number_value_after_colon(text, b, after) {
                return Some(v);
            }
        }
        from = after;
    }
    None
}

/// `key` matched at `at` is a real key, not the tail of another identifier: its preceding char
/// (if any) is not `.`, `_` or alphanumeric.
fn is_key_boundary_prev(b: &[u8], at: usize) -> bool {
    if at == 0 {
        return true;
    }
    let p = b[at - 1];
    p != b'.' && p != b'_' && !p.is_ascii_alphanumeric()
}

/// From `after` (just past a matched key), look for `: <digits+dot-run or "Infinity">` and
/// return the value as a string. `None` when there is no `:` or no value after it.
fn number_value_after_colon(text: &str, b: &[u8], after: usize) -> Option<String> {
    let i = skip_ws(b, after);
    if i >= b.len() || b[i] != b':' {
        return None;
    }
    let start = skip_ws(b, i + 1);
    read_number_or_infinity(text, b, start)
}

/// A digits+dot run at `start`, or `Infinity` (turbo-test#273: accept it as a no-timeout value).
fn read_number_or_infinity(text: &str, b: &[u8], start: usize) -> Option<String> {
    let mut j = start;
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
        j += 1;
    }
    if j > start {
        return Some(text[start..j].to_string());
    }
    text[start..]
        .starts_with("Infinity")
        .then(|| "Infinity".to_string())
}

/// The balanced `{...}`/`[...]` (or `(...)`) that OPENS at byte `open_at` — respecting nested
/// `{}[]()` and quoted strings — returned as `(inner_text, close_index)`. `None` if unterminated.
/// The close index lets a caller resume scanning PAST the block (so it can't re-match / loop).
pub(crate) fn balanced_block_end(text: &str, open_at: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    let open = b[open_at];
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        b'(' => b')',
        _ => return None,
    };
    let mut depth = 0i32;
    let mut i = open_at;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        if let Some(q) = quote {
            i += advance_in_quote(b[i], q, &mut quote);
            continue;
        }
        let c = b[i];
        match c {
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => {
                depth -= 1;
                if depth == 0 && c == close {
                    return Some((text[open_at + 1..i].to_string(), i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Inside a quoted run: consume the next byte(s), close the quote on `q`, return advance.
fn advance_in_quote(c: u8, q: u8, quote: &mut Option<u8>) -> usize {
    if c == b'\\' {
        return 2;
    }
    if c == q {
        *quote = None;
    }
    1
}

/// Lexically resolve `target` against `dir`: absolute stays, `.`/`..`-relative joins+normalizes,
/// a bare specifier (`react`, `@scope/x`) is returned unchanged (module-name alias).
fn resolve_alias_target(dir: &Path, target: &str) -> String {
    if target.starts_with('/') {
        return target.to_string();
    }
    if is_dot_relative(target) {
        return join_segments(dir, target);
    }
    target.to_string()
}

fn is_dot_relative(target: &str) -> bool {
    target.starts_with("./") || target.starts_with("../") || target == "." || target == ".."
}

fn join_segments(dir: &Path, target: &str) -> String {
    let mut out = dir.to_path_buf();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Split a top-level object/array's inner text into entries on TOP-LEVEL commas (respecting nested
/// brackets and quotes), so `{ a: f(x, y), b: 1 }` yields `a: f(x, y)` and `b: 1`, not four pieces.
fn split_top_level(inner: &str) -> Vec<String> {
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            b',' if depth == 0 => {
                out.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// Strip `//` line and `/* */` block comments from JS/TS config text WITHOUT touching comment-like
/// sequences inside string/template literals OR regex literals — so a commented-out `alias` block (a
/// `}`/`]` inside a comment) can't corrupt the brace scanners, and a config regex like
/// `p.replace(/^\/api\//, '/')` isn't mangled (its `\/` `/` must NOT read as a `//` comment).
/// Newlines are preserved. Regex-vs-division is disambiguated by the previous significant char.
pub(crate) fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut quote: Option<char> = None;
    let mut w = Words::default();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            step_in_quote(c, q, &mut chars, &mut out, &mut quote);
            continue;
        }
        step_outside_quote(c, &mut chars, &mut out, &mut quote, &mut w);
    }
    out
}

/// One `strip_comments` iteration while inside a quoted string/template literal.
fn step_in_quote(
    c: char,
    q: char,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    out: &mut String,
    quote: &mut Option<char>,
) {
    out.push(c);
    if c == '\\' {
        if let Some(n) = chars.next() {
            out.push(n);
        }
    } else if c == q {
        *quote = None;
    }
}

/// One `strip_comments` iteration while outside any quote.
fn step_outside_quote(
    c: char,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    out: &mut String,
    quote: &mut Option<char>,
    w: &mut Words,
) {
    match c {
        '\'' | '"' | '`' => {
            *quote = Some(c);
            out.push(c);
            w.reset(c);
        }
        '/' if chars.peek() == Some(&'/') => skip_line_comment(chars, out),
        '/' if chars.peek() == Some(&'*') => skip_block_comment(chars, out),
        // Regex literal (only in expression position): BLANK it (emit spaces). It must not be
        // copied verbatim — a regex body can contain `'`/`"`/`` ` `` (e.g. `/"/g`, `/'/g`), and
        // the downstream block scanners are quote-aware but NOT regex-aware, so a surviving quote
        // char would flip their string state and swallow the real `alias` block. Blanking keeps
        // the regex out of their way entirely.
        '/' if w.regex_starts() => {
            out.push(' ');
            blank_regex(chars, out);
            w.reset('/');
        }
        _ => {
            out.push(c);
            w.push(c);
        }
    }
}

/// Consume a `//` line comment; emit a single `\n` if we reach one.
fn skip_line_comment(chars: &mut std::iter::Peekable<std::str::Chars>, out: &mut String) {
    for n in chars.by_ref() {
        if n == '\n' {
            out.push('\n');
            break;
        }
    }
}

/// Consume a `/* … */` block comment (the leading `/` is already consumed); emit one space.
fn skip_block_comment(chars: &mut std::iter::Peekable<std::str::Chars>, out: &mut String) {
    chars.next();
    let mut prev = ' ';
    for n in chars.by_ref() {
        if prev == '*' && n == '/' {
            break;
        }
        prev = n;
    }
    out.push(' ');
}

/// The tokens before a `/` in [`strip_comments`], to tell a regex from division.
#[derive(Default)]
struct Words {
    // Previous significant (non-whitespace) char — updated for EVERY non-ws char, identifiers and
    // `)`/`]` included, so a `/` after a value token reads as DIVISION (not a regex). `cur_word` is
    // the identifier currently being read and `last_word` the one just completed (kept across a
    // following space) so a keyword-led regex (`return /re/`) is still recognized.
    prev_sig: char,
    cur_word: String,
    last_word: String,
    // Whether `cur_word` / `last_word` was reached by `.` property access (`obj.in`): a member NAME
    // is never a keyword, so a `/` after it is division.
    cur_member: bool,
    last_member: bool,
}

impl Words {
    /// After a string opener or a regex: `prev_sig` is `c`, no word pending.
    fn reset(&mut self, c: char) {
        self.prev_sig = c;
        self.cur_word.clear();
        self.last_word.clear();
    }

    fn regex_starts(&self) -> bool {
        is_regex_context(self.prev_sig)
            || keyword_before_slash(
                &self.cur_word,
                self.cur_member,
                &self.last_word,
                self.last_member,
            )
    }

    /// Any other char `c` copied through.
    fn push(&mut self, c: char) {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            self.push_word_char(c);
        } else if c.is_whitespace() {
            // Finalize the current word but keep it as `last_word` across the space, so
            // `return /re/` still sees the keyword. prev_sig (last non-ws char) is unchanged.
            self.flush_word_keeping_last();
        } else {
            // Punctuation: significant, and it ends any pending word.
            if self.cur_word.is_empty() {
                self.last_word.clear();
            } else {
                self.flush_word_keeping_last();
            }
            self.prev_sig = c;
        }
    }

    /// Identifier/number char: significant (a `/` after it is division), extends the current word.
    fn push_word_char(&mut self, c: char) {
        if self.cur_word.is_empty() {
            self.cur_member = self.prev_sig == '.';
        }
        self.cur_word.push(c);
        self.prev_sig = c;
    }

    fn flush_word_keeping_last(&mut self) {
        if !self.cur_word.is_empty() {
            self.last_word = std::mem::take(&mut self.cur_word);
            self.last_member = self.cur_member;
        }
    }
}

/// Blank a regex literal's body and flags-free close (after its opening `/`) into spaces.
/// Character classes `[...]` may hold an unescaped `/`, so track them to find the true close.
fn blank_regex(chars: &mut std::iter::Peekable<std::str::Chars>, out: &mut String) {
    let mut in_class = false;
    while let Some(n) = chars.next() {
        match n {
            // A regex literal cannot contain a raw newline. Hitting one means this `/`
            // was actually division misjudged as a regex — stop blanking at the line end
            // so a stray `/` can't consume (and drop) the rest of the config.
            '\n' => {
                out.push('\n');
                break;
            }
            '\\' => {
                if chars.next().is_some() {
                    out.push_str("  ");
                } else {
                    out.push(' ');
                }
            }
            '[' => {
                in_class = true;
                out.push(' ');
            }
            ']' => {
                in_class = false;
                out.push(' ');
            }
            '/' if !in_class => {
                out.push(' ');
                break;
            }
            _ => out.push(' '),
        }
    }
}

/// Keywords after which a `/` opens a regex literal, not division (`return /re/`, `typeof x || /re/`).
const REGEX_KEYWORDS: [&str; 12] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "do",
    "else",
    "yield",
    "await",
];

/// Whether the word right before a `/` is a keyword after which it opens a regex (`return /re/`):
/// the word being read wins over the one completed before the last space, and a word reached by
/// `.` property access (`obj.in / 2`) is a member name, never a keyword.
fn keyword_before_slash(cur: &str, cur_member: bool, last: &str, last_member: bool) -> bool {
    let (word, member) = if cur.is_empty() {
        (last, last_member)
    } else {
        (cur, cur_member)
    };
    !member && REGEX_KEYWORDS.contains(&word)
}

/// A `/` following one of these (or the start of input) opens a regex literal, not division.
fn is_regex_context(prev: char) -> bool {
    matches!(
        prev,
        '\0' | '('
            | '{'
            | '['
            | ','
            | ';'
            | ':'
            | '='
            | '!'
            | '&'
            | '|'
            | '?'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '^'
            | '<'
            | '>'
            | '~'
    )
}

/// Every `key: {..}|[..]` block in `text`, string-aware (run this on comment-stripped text). Unlike
/// scoping to a single `resolve`/`test` block, this finds ALL `alias` blocks wherever they appear —
/// a decoy `resolve: { conditions }` (no alias) before the real `resolve: { alias }` can't suppress
/// it — and skips `alias`-looking text inside string literals. Advances past each block's close, so
/// it can't loop (including on an empty `{}` / `[]`). Blocks come back in source order.
fn find_key_blocks(text: &str, key: &str) -> Vec<String> {
    let b = text.as_bytes();
    let kb = key.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                quote = Some(c);
                i += 1;
            }
            _ if b[i..].starts_with(kb) => match key_block_at(text, i, kb.len()) {
                Some((inner, end)) => {
                    out.push(inner);
                    i = end + 1;
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    out
}

/// The block of the key (`len` bytes) at `i` when it is a whole name followed by `: {`/`: [`:
/// (its inner text, the offset of its close).
fn key_block_at(text: &str, i: usize, len: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    if !is_word_boundary_before(b, i) {
        return None;
    }
    let j = skip_ws(b, i + len);
    if j >= b.len() || b[j] != b':' {
        return None;
    }
    let k = skip_ws(b, j + 1);
    if k < b.len() && (b[k] == b'{' || b[k] == b'[') {
        balanced_block_end(text, k)
    } else {
        None
    }
}

/// Whether the byte before offset `i` is not part of an identifier (so `i` starts a fresh word).
fn is_word_boundary_before(b: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let p = b[i - 1];
    p != b'.' && p != b'_' && p != b'$' && !p.is_ascii_alphanumeric()
}

/// Parse `resolve.alias` and `test.alias` into `(find, resolved-target)` pairs. Supports the object
/// form (`{ '@': path.resolve(__dirname, './src') }`) and the array form
/// (`[{ find: '@', replacement: '/abs/src' }]`). Comments are stripped first, then EVERY `alias:`
/// block is parsed (string-aware, so `alias`-looking text inside a string/comment is ignored, and a
/// decoy `resolve: { conditions }` before the real `resolve: { alias }` can't suppress it). Blocks
/// are applied in source order with later-wins, so a `test.alias` placed after `resolve.alias`
/// (the conventional order) overrides it on a key collision. Values are arbitrary JS, so the target
/// is derived heuristically (see `alias_target`); a regex-literal `find` is skipped, a quoted one is
/// a prefix string whatever its first char (`'/@z'`).
fn config_aliases(start_dir: &Path, forced: Option<&Path>) -> Vec<(String, String)> {
    let Some(cfg) = find_config(start_dir, forced) else {
        return vec![];
    };
    let dir = &cfg.dir;
    let text = strip_comments(&cfg.text);
    let mut map: Vec<(String, String)> = Vec::new();
    let mut put = |k: String, v: String| {
        if let Some(e) = map.iter_mut().find(|(ek, _)| *ek == k) {
            e.1 = v;
        } else {
            map.push((k, v));
        }
    };
    for inner in find_key_blocks(&text, "alias") {
        for (k, v) in parse_alias_inner(&inner, dir) {
            put(k, v);
        }
    }
    map
}

fn parse_alias_inner(inner: &str, dir: &Path) -> Vec<(String, String)> {
    // Array form (`alias: [ { find, replacement }, … ]`): the inner text begins with an object `{`.
    // The object form's inner begins with a quoted/identifier key instead — so the first char
    // distinguishes them without a fragile substring test.
    if inner.trim().starts_with('{') {
        return parse_alias_array(inner, dir);
    }
    parse_alias_object(inner, dir)
}

/// Array form: `alias: [ { find, replacement }, … ]`.
fn parse_alias_array(inner: &str, dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in split_top_level(inner) {
        let e = entry.trim();
        let e = e.strip_prefix('{').unwrap_or(e);
        let e = e.strip_suffix('}').unwrap_or(e);
        // Only a quoted string `find` is a (prefix) alias, whatever its first char (`'/@z'`);
        // a regex literal `find` (unquoted — already blanked by strip_comments) is skipped.
        let find = value_after_key(e, "find")
            .filter(|v| v.starts_with(['\'', '"', '`']))
            .and_then(|v| first_quoted(&v));
        let repl = value_after_key(e, "replacement").and_then(|v| alias_target(dir, &v));
        if let (Some(f), Some(r)) = (find, repl) {
            out.push((f, r));
        }
    }
    out
}

/// Object form: `alias: { key: value, … }`.
fn parse_alias_object(inner: &str, dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in split_top_level(inner) {
        let Some(colon) = top_level_colon(&entry) else {
            continue;
        };
        let key_raw = entry[..colon].trim();
        let val = &entry[colon + 1..];
        let Some(key) = alias_object_key(key_raw) else {
            continue;
        };
        if let Some(target) = alias_target(dir, val) {
            out.push((key, target));
        }
    }
    out
}

/// A plain identifier or quoted alias key; `None` for a `[computed]` or regex-literal key.
fn alias_object_key(key_raw: &str) -> Option<String> {
    if let Some(q) = first_quoted(key_raw) {
        return Some(q);
    }
    (!key_raw.is_empty() && !key_raw.starts_with('[') && !key_raw.starts_with('/'))
        .then(|| key_raw.to_string())
}

/// Derive the alias target path/specifier from a JS value expression. When the value looks like a
/// filesystem-path expression (`path.resolve`/`path.join`/`fileURLToPath`/`new URL`/`__dirname`/
/// `import.meta.url`/`import.meta.dirname`), ALL its quoted segments are joined against the config
/// dir with path.resolve semantics (so `path.resolve(__dirname, '../..', 'pkg/src')` resolves
/// correctly, not just its last segment). Otherwise the value is a plain literal (`'/abs'`, `'./rel'`, or a bare module specifier
/// like `'preact/compat'`) and its last string is resolved by its own prefix. A bare `__dirname` /
/// `import.meta.dirname` base is the config dir itself. `None` if there is no literal otherwise.
fn alias_target(dir: &Path, val: &str) -> Option<String> {
    const PATH_MARKERS: [&str; 7] = [
        "path.resolve",
        "path.join",
        "fileURLToPath",
        "new URL",
        "__dirname",
        "import.meta.url",
        "import.meta.dirname",
    ];
    // A value can be a string CONCATENATION (`path.resolve(__dirname,'src') + '/sub'`). Split on the
    // top-level `+`: the FIRST operand is the base (path.resolve-joined when it's a path expression,
    // else its plain literal), and each later operand is a plain string appended LITERALLY — so
    // `+ '/sub'` extends the path and a trailing `+ '/'` is a separator, NOT a reset-to-root (which
    // is what path.resolve semantics on a `'/'` argument would wrongly do).
    let mut parts = split_top_level_plus(val).into_iter();
    let first = parts.next()?;
    let mut base = alias_base(dir, &first, &PATH_MARKERS)?;
    for p in parts {
        for s in extract_quoted(&p) {
            base.push_str(&s);
        }
    }
    Some(base)
}

/// The base path for the first `+`-operand of an alias value (path expression or plain literal).
fn alias_base(dir: &Path, first: &str, markers: &[&str]) -> Option<String> {
    // Test markers against CODE (strings blanked) so a literal merely CONTAINING `__dirname` etc.
    // isn't mistaken for a path expression.
    if markers.iter().any(|m| blank_quoted(first).contains(m)) {
        let quoted = extract_quoted(first);
        // a bare config-dir base (`__dirname + '/z'`) is the dir itself: joining no segments
        let bare_dir = matches!(first, "__dirname" | "import.meta.dirname");
        if quoted.is_empty() && !bare_dir {
            return None;
        }
        return Some(resolve_alias_join(dir, &quoted));
    }
    Some(resolve_alias_target(dir, extract_quoted(first).last()?))
}

/// Split `s` on TOP-LEVEL `+` (string concatenation), not inside quotes/brackets. Operands trimmed.
fn split_top_level_plus(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'+' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(s[start..].trim().to_string());
    out
}

/// Replace every quoted-string span in `s` with spaces (keeping length/positions) so a marker/token
/// search sees only the surrounding CODE, never string contents.
fn blank_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => blank_quoted_in_string(c, q, &mut chars, &mut quote, &mut out),
            None => blank_quoted_in_code(c, &mut quote, &mut out),
        }
    }
    out
}

fn blank_quoted_in_string(
    c: char,
    q: char,
    chars: &mut std::str::Chars,
    quote: &mut Option<char>,
    out: &mut String,
) {
    out.push(' ');
    if c == '\\' {
        if chars.next().is_some() {
            out.push(' ');
        }
    } else if c == q {
        *quote = None;
    }
}

fn blank_quoted_in_code(c: char, quote: &mut Option<char>, out: &mut String) {
    if c == '\'' || c == '"' || c == '`' {
        *quote = Some(c);
        out.push(' ');
    } else {
        out.push(c);
    }
}

/// Join path segments against `dir` with Node `path.resolve` semantics: an absolute segment resets
/// the accumulator, a relative one is appended lexically (`.`/`..` collapsed).
fn resolve_alias_join(dir: &Path, segs: &[String]) -> String {
    let mut cur = dir.to_path_buf();
    for seg in segs {
        if seg.starts_with('/') {
            cur = PathBuf::from(seg);
            continue;
        }
        for part in seg.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    cur.pop();
                }
                p => cur.push(p),
            }
        }
    }
    cur.to_string_lossy().into_owned()
}

/// The FIRST quoted string in `s` (used for an alias key / a `find:` value).
fn first_quoted(s: &str) -> Option<String> {
    extract_quoted(s).into_iter().next()
}

/// Byte offset of the first TOP-LEVEL `:` in `entry` (not inside quotes/brackets).
fn top_level_colon(entry: &str) -> Option<usize> {
    let b = entry.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The value expression for `key:` within an object entry `e`, bounded at the entry's NEXT top-level
/// comma. Scans all occurrences of `key` and accepts the first that is a real object key (word
/// boundary before, `:` after optional whitespace) — so a VALUE containing the substring
/// `find`/`replacement` doesn't hijack the match. The comma bound matters for the ARRAY form, where a
/// whole `{ find: X, replacement: Y }` object is one entry: without it, `value_after_key(e,"find")`
/// would read across the comma into `replacement` (grabbing its string, or — when `find` is a regex
/// blanked to spaces — mis-picking `replacement`'s string as the find key).
fn value_after_key(e: &str, key: &str) -> Option<String> {
    let b = e.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = e[from..].find(key) {
        let at = from + rel;
        let after = at + key.len();
        if value_key_prev_ok(b, at) {
            if let Some(v) = value_after_key_at(e, b, after) {
                return Some(v);
            }
        }
        from = after;
    }
    None
}

/// Word-boundary check: the byte before an object-key match is not an identifier character.
fn value_key_prev_ok(b: &[u8], at: usize) -> bool {
    if at == 0 {
        return true;
    }
    let p = b[at - 1];
    p != b'.' && p != b'_' && p != b'$' && !p.is_ascii_alphanumeric()
}

/// After a potential key match at `after`: the trimmed value if the next non-ws byte is `:`.
fn value_after_key_at(e: &str, b: &[u8], after: usize) -> Option<String> {
    let ws = skip_ws(b, after);
    if ws >= b.len() || b[ws] != b':' {
        return None;
    }
    let val_start = ws + 1;
    let rel_end = top_level_comma(&e[val_start..]).unwrap_or(e.len() - val_start);
    Some(e[val_start..val_start + rel_end].trim().to_string())
}

/// Byte offset of the first TOP-LEVEL `,` in `s` (not inside quotes/brackets). `None` if none.
fn top_level_comma(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether test file `t` (absolute) passes the positional filter `f`, as vitest's filterFiles:
/// an absolute filter that is a prefix of the path, or the root-relative path containing the
/// filter, or the filter made root-relative (from the process's cwd; a trailing slash kept),
/// case-insensitively.
fn filter_matches(root: &Path, proc_cwd: &Path, f: &str, t: &Path) -> bool {
    let rel = |p: &Path| -> String {
        let r = p.strip_prefix(root).unwrap_or(p);
        r.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    };
    let test_file = rel(t).to_lowercase();
    if Path::new(f).is_absolute() && t.to_string_lossy().starts_with(f) {
        return true;
    }
    let mut relative = rel(&absolute_in(proc_cwd, Path::new(f)));
    if f.ends_with('/') {
        relative.push('/');
    }
    test_file.contains(&f.to_lowercase()) || test_file.contains(&relative.to_lowercase())
}

/// Whether an exclude glob covers every path under directory `rel` (a `<prefix>/**` glob whose
/// prefix matches it), so discovery need not descend into it.
fn excludes_dir(exclude: &[String], rel: &str) -> bool {
    exclude.iter().any(|g| {
        g.strip_suffix("/**")
            .is_some_and(|prefix| glob_matches(prefix, rel))
    })
}

/// Default discovery: all test files under `cwd` matching the config's `test.include` (else vitest's
/// default include) and not its `test.exclude` (else vitest's default exclude, which is what skips
/// `node_modules` and `.git`: a config exclude replaces it, turbo-test#63). Globs match the path
/// relative to the config's directory (else `cwd`). Mirrors cli.js `discover`.
fn discover(cwd: &Path, forced_config: Option<&Path>) -> Vec<PathBuf> {
    let mut all = Vec::new();
    let pats = vitest_patterns(cwd, forced_config);
    let jest_match = if pats.is_none() { jest_test_match(cwd) } else { None };
    let base = pats.as_ref().map_or(cwd, |p| p.root.as_path());
    let include = pats.as_ref().and_then(|p| p.include.as_deref());
    let exclude: Vec<String> = match pats.as_ref().and_then(|p| p.exclude.clone()) {
        Some(e) => e,
        None => DEFAULT_EXCLUDE_GLOBS
            .iter()
            .map(|g| g.to_string())
            .collect(),
    };
    let rel = |f: &Path| -> String {
        // base-relative POSIX path (vitest matches globs against this).
        let r = f.strip_prefix(base).unwrap_or(f);
        r.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    };
    let prune = |d: &Path| excludes_dir(&exclude, &rel(d));
    let has_include = include.is_some() || jest_match.is_some();
    if has_include {
        walk(cwd, &mut all, &|_| true, &prune);
    } else {
        walk(cwd, &mut all, &is_test_file, &prune);
    }
    all.retain(|f| {
        let r = rel(f);
        let inc_ok = if let Some(inc) = include {
            inc.iter().any(|g| glob_matches(g, &r))
        } else if let Some(ref jm) = jest_match {
            jm.iter().any(|g| glob_matches(g, &r))
        } else {
            true
        };
        inc_ok && !exclude.iter().any(|g| glob_matches(g, &r))
    });
    all.sort();
    all
}

const JEST_CONFIG_NAMES: [&str; 5] = [
    "jest.config.js",
    "jest.config.cjs",
    "jest.config.mjs",
    "jest.config.ts",
    "jest.config.json",
];

fn jest_test_match(cwd: &Path) -> Option<Vec<String>> {
    let text = jest_config_text_in(cwd)?;
    scan_string_array(&text, "testMatch")
}

fn jest_config_text_in(dir: &Path) -> Option<String> {
    for n in &JEST_CONFIG_NAMES {
        if let Ok(s) = std::fs::read_to_string(dir.join(n)) {
            return Some(s);
        }
    }
    std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|pkg| package_json_jest_block(&pkg))
}

fn package_json_jest_block(pkg: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(pkg).ok()?;
    let jest = v.get("jest")?;
    jest.is_object().then(|| jest.to_string())
}

/// `--changed [since]`: absolute paths git reports changed vs `since` (default working-tree vs
/// HEAD/index) — staged + unstaged + untracked. `None` when git is unavailable / not a repo
/// (caller then runs everything). Mirrors cli.js `gitChanged`.
fn git_changed(since: &str, cwd: &Path) -> Option<HashSet<PathBuf>> {
    let run = |args: &[&str]| -> Option<Vec<String>> { run_git_lines(args, cwd) };
    let root_lines = run(&["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(root_lines.first()?);
    let diff_ref: Vec<&str> = git_diff_ref_args(since);
    let mut cmds: Vec<Vec<&str>> = vec![
        {
            let mut v = vec!["diff", "--name-only"];
            v.extend(&diff_ref);
            v
        },
        {
            let mut v = vec!["diff", "--name-only", "--cached"];
            v.extend(&diff_ref);
            v
        },
        vec!["ls-files", "--others", "--exclude-standard"],
    ];
    let mut set = HashSet::new();
    for args in cmds.drain(..) {
        collect_git_lines(run(&args), &repo_root, &mut set);
    }
    Some(set)
}

fn git_diff_ref_args(since: &str) -> Vec<&str> {
    if !since.is_empty() && since != "true" {
        vec![since]
    } else {
        vec![]
    }
}

fn collect_git_lines(lines: Option<Vec<String>>, repo_root: &Path, set: &mut HashSet<PathBuf>) {
    let Some(lines) = lines else { return };
    for f in lines {
        set.insert(repo_root.join(f));
    }
}

/// `git <args>` in `cwd`: each stdout line (dropping blanks). `None` on non-zero exit or spawn error.
fn run_git_lines(args: &[&str], cwd: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect(),
    )
}

/// Launcher-consumed options.
struct Opts {
    config: Option<String>,
    root: Option<String>,
    /// `--root` (not `--dir`): the files are named relative to it.
    name_root: Option<String>,
    environment: Option<String>,
    /// `Some("true")` for bare `--changed`, `Some(<ref>)` for `--changed <ref>`, `None` if absent.
    changed: Option<String>,
    isolate: Option<bool>,
    pass_with_no_tests: bool,
}

/// Whether `a` is a `--key` / `-k` whose VALUE is the following token (so we don't split a file off
/// as its value when forwarding).
fn flag_takes_value(a: &str) -> bool {
    VALUE_FLAGS.contains(&a)
}

/// Whether the forwarded-flag list already carries `needle` (bare or `needle=…` inline form) — so
/// a user-passed coverage flag wins over the config-injected default.
fn has_flag(forward: &[String], needle: &str) -> bool {
    forward
        .iter()
        .any(|f| f == needle || f.starts_with(&format!("{needle}=")))
}

/// `p` made absolute against `cwd` (the process cwd) — what `std::path::absolute(p)` yields when
/// `cwd` is the current directory, falling back to `p` itself when that fails (an empty path).
fn absolute_in(cwd: &Path, p: &Path) -> PathBuf {
    let joined = if p.as_os_str().is_empty() || p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    std::path::absolute(&joined).unwrap_or_else(|_| p.to_path_buf())
}

/// What `prepare` does, computed by `plan` without side effects.
#[derive(Debug, PartialEq)]
pub struct Plan {
    /// Env vars to set (in order) for the runner.
    pub env: Vec<(&'static str, String)>,
    /// Lines to print to stderr (in order).
    pub messages: Vec<String>,
    /// `Ok(argv)` — the effective argv to run with — or `Err(code)` to exit with.
    pub outcome: Result<Vec<String>, i32>,
}

/// Port of cli.js `main()` up to the point of spawning the native binary. Returns the effective
/// argv (forwarded runner flags + resolved absolute test files) for `cli::parse_args` to parse, or
/// exits the process for the terminal cases. `raw` is argv WITHOUT the program name.
pub fn prepare(raw: Vec<String>) -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let plan = plan(raw, &cwd);
    for (k, v) in &plan.env {
        std::env::set_var(k, v);
    }
    for m in &plan.messages {
        eprintln!("{m}");
    }
    match plan.outcome {
        Ok(argv) => argv,
        Err(code) => std::process::exit(code),
    }
}

/// The side-effect-free body of `prepare`: `proc_cwd` is the process cwd (relative file args,
/// `-c` and `--root` resolve against it). Env vars / stderr lines / the exit decision are returned.
pub fn plan(mut raw: Vec<String>, proc_cwd: &Path) -> Plan {
    let mut env: Vec<(&'static str, String)> = Vec::new();
    let mut messages: Vec<String> = Vec::new();
    // Accept-and-strip a leading vitest subcommand (`run`/`watch`/`dev`) — turbo-test is always a
    // single run; letting it reach the runner would look like a phantom test-file path.
    if raw
        .first()
        .map(|s| matches!(s.as_str(), "run" | "watch" | "dev"))
        .unwrap_or(false)
    {
        raw.remove(0);
    }

    let mut opts = Opts {
        config: None,
        root: None,
        name_root: None,
        environment: None,
        changed: None,
        isolate: None,
        pass_with_no_tests: false,
    };
    let mut forward: Vec<String> = Vec::new(); // runner-side flags, forwarded verbatim
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < raw.len() {
        i = plan_arg(&raw, i, &mut opts, &mut forward, &mut files, &mut messages) + 1;
    }

    // Discovery root: --root/--dir override cwd.
    let cwd: PathBuf = match &opts.root {
        Some(r) => absolute_in(proc_cwd, Path::new(r)),
        None => proc_cwd.to_path_buf(),
    };
    let forced = opts
        .config
        .as_ref()
        .map(|f| absolute_in(proc_cwd, Path::new(f)));

    config_env(&opts, proc_cwd, &cwd, forced.as_deref(), &mut env);
    config_forward(&cwd, forced.as_deref(), &mut forward);

    // Resolve the test file set: the discovered files, named relative to the process cwd as
    // vitest's (root-relative) names. Positional arguments are filters, as vitest's: the
    // discovered files whose root-relative path contains one (directories and file paths alike).
    let mut test_files: Vec<PathBuf> = discover(&cwd, forced.as_deref())
        .into_iter()
        .filter(|t| files.is_empty() || files.iter().any(|f| filter_matches(&cwd, proc_cwd, f, t)))
        .map(|t| t.strip_prefix(proc_cwd).map(Path::to_path_buf).unwrap_or(t))
        .collect();
    if test_files.is_empty() {
        // vitest: exit 1 (0 with --passWithNoTests), the filters, include and exclude
        let code = i32::from(!opts.pass_with_no_tests);
        messages.push(no_files_message(code, &files, &cwd, forced.as_deref()));
        return Plan {
            env,
            messages,
            outcome: Err(code),
        };
    }

    // `--changed [since]`: keep only discovered test files git reports changed. Direct changed-file
    // filter (no import graph). Nothing changed → running zero tests is expected → exit 0.
    if let Some(since) = &opts.changed {
        if let Some(code) = keep_changed(since, &cwd, proc_cwd, &mut test_files, &mut messages) {
            return Plan {
                env,
                messages,
                outcome: Err(code),
            };
        }
    }

    // Effective argv for the runner loop: forwarded flags, then resolved files.
    let mut out = forward;
    out.extend(test_files.into_iter().map(|f| f.display().to_string()));
    Plan {
        env,
        messages,
        outcome: Ok(out),
    }
}

/// `plan`'s handling of the argument at `i` (into `opts`, `forward`, `files`, `messages`); the
/// index of the last token it consumed.
fn plan_arg(
    raw: &[String],
    mut i: usize,
    opts: &mut Opts,
    forward: &mut Vec<String>,
    files: &mut Vec<String>,
    messages: &mut Vec<String>,
) -> usize {
    let a = raw[i].clone();
    // `--key=value` inline form for launcher value-flags.
    let (key, inline_val): (&str, Option<String>) = match a.find('=') {
        Some(eq) if a.starts_with("--") => (&a[..eq], Some(a[eq + 1..].to_string())),
        _ => (a.as_str(), None),
    };
    if launcher_flag(key, opts, &mut || take_val(raw, &mut i, &inline_val)) {
        return i;
    }
    if let Some(end) = handle_dotted_vitest_flag(raw, i, &a, key, &inline_val, forward) {
        return end;
    }
    if !flag_takes_value(&a) {
        push_plain_arg(a, forward, files);
        return i;
    }
    forward_value_flag(raw, i, a, forward, messages)
}

/// `plan_arg`'s dotted-vitest-flag arm (`--sequence.seed 123`, `--browser.name chromium`, …).
/// Drops the flag and, when the next token is its value (a non-flag), that too — otherwise the
/// orphaned value token would be mistaken for a test-file path and hard-fail the run. Returns the
/// new `i` when this arm matched, `None` to fall through.
fn handle_dotted_vitest_flag(
    raw: &[String],
    mut i: usize,
    a: &str,
    key: &str,
    inline_val: &Option<String>,
    forward: &mut Vec<String>,
) -> Option<usize> {
    if a == SEQUENCE_CONCURRENT {
        forward.push(a.to_string());
        return Some(i);
    }
    if key.starts_with("--sequence") || key.starts_with("--browser") || key.starts_with("--inspect")
    {
        let _ = take_val(raw, &mut i, inline_val);
        return Some(i);
    }
    None
}

/// Dispatch a non-value-taking argument: a flag goes to `forward`, a non-empty bare token to
/// `files`. An empty token (a stray `""` from a shell wrapper) is dropped — as a filter it would
/// match every test file.
fn push_plain_arg(a: String, forward: &mut Vec<String>, files: &mut Vec<String>) {
    if a.starts_with('-') {
        forward.push(a);
    } else if !a.is_empty() {
        files.push(a);
    }
}

/// Forward a value-taking flag `a` at `i`, consuming its following value when one is present
/// (so the runner does not misread a test-file path as the value). Returns the new `i`. When no
/// value follows, the flag is dropped with a message (a `--coverage-*` one still implies
/// `--coverage`, as it does in the runner).
fn forward_value_flag(
    raw: &[String],
    i: usize,
    a: String,
    forward: &mut Vec<String>,
    messages: &mut Vec<String>,
) -> usize {
    if i + 1 < raw.len() && !raw[i + 1].starts_with('-') {
        forward.push(a);
        forward.push(raw[i + 1].clone());
        return i + 1;
    }
    messages.push(format!("turbo-test: ignoring '{a}': missing value"));
    if a.starts_with("--coverage") {
        forward.push("--coverage".to_string());
    }
    i
}

/// The value of the launcher value-flag at `*i`: inline `=v`, else the next non-flag token
/// (`*i` then moves onto it).
fn take_val(raw: &[String], i: &mut usize, inline: &Option<String>) -> Option<String> {
    if let Some(v) = inline.clone() {
        return Some(v);
    }
    if *i + 1 < raw.len() && !raw[*i + 1].starts_with('-') {
        *i += 1;
        return Some(raw[*i].clone());
    }
    None
}

/// A launcher flag `key` applied to `opts` (its value from `take_val`); whether it was one.
fn launcher_flag(key: &str, opts: &mut Opts, take_val: &mut dyn FnMut() -> Option<String>) -> bool {
    match key {
        "--passWithNoTests" => opts.pass_with_no_tests = true,
        "-c" | "--config" => opts.config = take_val(),
        // vitest distinguishes --root (project root) from --dir (test scan dir); turbo-test
        // scans the project root, so both override the discovery directory (last wins). Only
        // --root moves the base the files are named from (vitest's root).
        "--root" | "--dir" => {
            opts.root = take_val();
            if key == "--root" {
                opts.name_root = opts.root.clone();
            }
        }
        "--environment" => opts.environment = take_val(),
        // `--changed`'s arg is OPTIONAL: a following non-flag token is the `since` ref.
        "--changed" => opts.changed = Some(take_val().unwrap_or_else(|| "true".to_string())),
        "--isolate" => opts.isolate = Some(true),
        "--no-isolate" => opts.isolate = Some(false),
        // Globals are ALWAYS on in turbo-test; accept both spellings as no-ops.
        "--globals" | "--no-globals" => {}
        // vitest VALUE-taking flags with no effect on a native single-run runner. Consume the
        // flag AND its value HERE (inline `=v` or the next non-flag token) and drop both, so the
        // runner never sees them — the runner must NOT re-consume a value that may be absent
        // (a trailing `--pool`) and swallow a real test-file arg instead. `--no-file-parallelism`
        // is intentionally NOT here: it maps to real behavior (jobs=1) and is forwarded below.
        "--pool" | "--mode" | "--project" | "--exclude" => {
            let _ = take_val();
        }
        // Boolean / no-effect vitest flags — dropped (NO value consumed, so a following test
        // file isn't eaten).
        "--run"
        | "--watch"
        | "-w"
        | "--no-watch"
        | "--logHeapUsage"
        | "--hideSkippedTests"
        | "--disableConsoleIntercept"
        | "--dom"
        | "--segfaultRetry"
        | "--printConsoleTrace"
        | "--color"
        | "--no-color"
        | "--inspect"
        | "--inspect-brk" => {}
        _ => return false,
    }
    true
}

/// The runner env `plan` derives from `opts` and the vitest config.
fn config_env(
    opts: &Opts,
    proc_cwd: &Path,
    cwd: &Path,
    forced: Option<&Path>,
    env: &mut Vec<(&'static str, String)>,
) {
    push_opts_env(opts, proc_cwd, env);
    push_config_env(opts, cwd, forced, env);
}

/// Opts-only switches: `--root`, `--passWithNoTests`, `--isolate`.
fn push_opts_env(opts: &Opts, proc_cwd: &Path, env: &mut Vec<(&'static str, String)>) {
    // `--root`: the reports name the files relative to it (cli.rs `display_names`).
    if let Some(r) = &opts.name_root {
        env.push((
            "TURBO_ROOT",
            absolute_in(proc_cwd, Path::new(r)).display().to_string(),
        ));
    }

    // `--passWithNoTests` also covers a test file that collects no test at all (vitest: no
    // "No test suite found in file" error) → the runtime reads it per file.
    if opts.pass_with_no_tests {
        env.push(("TURBO_PASS_WITH_NO_TESTS", "1".to_string()));
    }

    // `--isolate` / `--no-isolate` → the runner's reuse env switches (read per-file in runner.rs).
    match opts.isolate {
        Some(false) => env.push(("TURBO_REUSE_ISOLATE", "1".to_string())),
        Some(true) => env.push(("TURBO_NO_REUSE", "1".to_string())),
        None => {}
    }
}

/// Config-fed env: `environment`, `server.deps.inline`, aliases, `includeTaskLocation`.
fn push_config_env(
    opts: &Opts,
    cwd: &Path,
    forced: Option<&Path>,
    env: &mut Vec<(&'static str, String)>,
) {
    // `--environment` → TURBO_ENV (falls back to config `test.environment`). A per-file pragma
    // overrides this at runtime (runner.rs).
    let environment = opts
        .environment
        .clone()
        .or_else(|| find_config(cwd, forced).and_then(|c| config_environment(&c.text)));
    if let Some(e) = environment {
        env.push(("TURBO_ENV", e));
    }

    // `server.deps.inline` → TURBO_DEPS_INLINE: the packages whose own imports see vi.mock
    // (vitest runs them through its module runner, turbo-test#216). `*` = `inline: true`.
    let inline = find_config(cwd, forced)
        .map(|c| config_deps_inline(&c.text))
        .unwrap_or_default();
    if !inline.is_empty() {
        env.push(("TURBO_DEPS_INLINE", inline.join("\u{1f}")));
    }

    // `resolve.alias` / `test.alias` (incl. what plugins like vite-tsconfig-paths' non-tsconfig
    // fallback emit) → TURBO_ALIAS, read by the runner's module resolver. Serialized as
    // `find\x1ftarget` records joined by `\x1e`. tsconfig `paths` are handled separately (native).
    let aliases = config_aliases(cwd, forced);
    if !aliases.is_empty() {
        let ser = aliases
            .iter()
            .map(|(k, v)| format!("{k}\u{1f}{v}"))
            .collect::<Vec<_>>()
            .join("\u{1e}");
        env.push(("TURBO_ALIAS", ser));
    }
    // `test.includeTaskLocation: true` → test locations in the JSON report, like the
    // `--includeTaskLocation` flag (which the runner reads itself).
    if find_config(cwd, forced).is_some_and(|c| config_true(&c.text, "includeTaskLocation")) {
        env.push(("TURBO_TASK_LOCATION", "1".to_string()));
    }
}

/// The runner flags `plan` fills from the vitest config unless passed explicitly: coverage
/// thresholds/include/exclude and the per-test timeout.
fn config_forward(cwd: &Path, forced: Option<&Path>, forward: &mut Vec<String>) {
    // Coverage: fill thresholds/include/exclude from vitest config unless passed explicitly.
    let coverage_on = forward.iter().any(|f| f.starts_with("--coverage"));
    if coverage_on {
        if let Some(cov) = vitest_coverage(cwd, forced) {
            coverage_forward(&cov, forward);
        }
    }
    forward_config_numbers(cwd, forced, forward);
    // `test.sequence.concurrent: true` (turbo-test#256)
    let seq = find_config(cwd, forced)
        .and_then(|c| scan_braced(&c.text, "sequence"))
        .is_some_and(|b| config_true(&b, "concurrent"));
    if seq && !has_flag(forward, SEQUENCE_CONCURRENT) {
        forward.push(SEQUENCE_CONCURRENT.to_string());
    }
}

/// Per-test timeout: honor the vitest config `test.testTimeout` unless the user passed
/// `--testTimeout` explicitly. Same for `hookTimeout` (turbo-test#255) and `maxConcurrency` (#256).
fn forward_config_numbers(cwd: &Path, forced: Option<&Path>, forward: &mut Vec<String>) {
    for key in ["testTimeout", "hookTimeout", "maxConcurrency"] {
        let flag = format!("--{key}");
        if has_flag(forward, &flag) {
            continue;
        }
        if let Some(v) = find_config(cwd, forced).and_then(|c| scan_number(&c.text, key)) {
            forward.push(flag);
            forward.push(v);
        }
    }
}

/// vitest's `--sequence.concurrent` (every test concurrent): forwarded to the runner.
const SEQUENCE_CONCURRENT: &str = "--sequence.concurrent";

/// Coverage thresholds/include/exclude from the config `cov`, unless `forward` passes them.
fn coverage_forward(cov: &CoverageCfg, forward: &mut Vec<String>) {
    forward_coverage_thresholds(cov.thresholds.as_ref(), forward);
    forward_coverage_globs(&cov.include, "--coverage-include", forward);
    forward_coverage_globs(&cov.exclude, "--coverage-exclude", forward);
}

fn forward_coverage_thresholds(thr: Option<&String>, forward: &mut Vec<String>) {
    let Some(thr) = thr else { return };
    if has_flag(forward, "--coverage-thresholds") || has_flag(forward, "--coverage-threshold") {
        return;
    }
    forward.push("--coverage-thresholds".to_string());
    forward.push(thr.clone());
}

fn forward_coverage_globs(globs: &[String], flag: &str, forward: &mut Vec<String>) {
    if globs.is_empty() || has_flag(forward, flag) {
        return;
    }
    forward.push(flag.to_string());
    forward.push(globs.join(","));
}

/// The "No test files found" report: the exit `code`, the filters, include and exclude.
fn no_files_message(code: i32, files: &[String], cwd: &Path, forced: Option<&Path>) -> String {
    let mut msg = format!("No test files found, exiting with code {code}\n\n");
    if !files.is_empty() {
        msg.push_str(&format!("filter: {}\n", files.join(", ")));
    }
    let pats = vitest_patterns(cwd, forced);
    let list =
        |l: Option<&Vec<String>>, default: &str| l.map_or(default.to_string(), |l| l.join(", "));
    let include = list(
        pats.as_ref().and_then(|p| p.include.as_ref()),
        DEFAULT_INCLUDE,
    );
    let exclude = list(
        pats.as_ref().and_then(|p| p.exclude.as_ref()),
        DEFAULT_EXCLUDE,
    );
    msg.push_str(&format!("include: {include}\nexclude:  {exclude}"));
    msg
}

/// `--changed [since]`: keep only discovered test files git reports changed. Direct changed-file
/// filter (no import graph). Nothing changed → running zero tests is expected → `Some(0)`.
fn keep_changed(
    since: &str,
    cwd: &Path,
    proc_cwd: &Path,
    test_files: &mut Vec<PathBuf>,
    messages: &mut Vec<String>,
) -> Option<i32> {
    let Some(changed) = git_changed(since, cwd) else {
        messages.push(
            "turbo-test: --changed: git unavailable / not a repo — running all discovered tests."
                .to_string(),
        );
        return None;
    };
    let before = test_files.len();
    test_files.retain(|f| {
        let abs = absolute_in(proc_cwd, f);
        changed.contains(&abs)
    });
    if test_files.is_empty() {
        messages.push(format!(
            "turbo-test: --changed: no changed test files (of {before}) — exiting 0."
        ));
        return Some(0);
    }
    None
}

#[cfg(all(test, unix))]
#[path = "launcher_tests.rs"]
mod tests;
