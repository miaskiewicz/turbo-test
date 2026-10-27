//! The error vitest fails a JavaScript module that does not parse with (turbo-test#108).
//!
//! vite transforms a `.js` / `.mjs` / `.cjs` module in two steps that parse it: the import
//! analysis lexes it with es-module-lexer (a source the lexer rejects — an unbalanced `)` / `}`,
//! an unterminated string, template or regular expression — fails with vite's "Failed to parse
//! source for import analysis …"), then the SSR transform parses it with rolldown's oxc parser
//! (any other syntax error is rolldown's `Parse failure: Parse failed with N error(s): …` with a
//! code frame and the module's root-relative URL). turbo-test asks for this only after V8 failed
//! to compile the module: a source oxc parses is left with V8's SyntaxError (an error only V8
//! reports, as in vitest; turbo-test#99).

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// The (error name, message) vitest fails the module `path` (source `src`, project root `root`)
/// with, when `path` is a JavaScript module oxc does not parse.
pub fn vite_parse_error(path: &Path, src: &str, root: &Path) -> Option<(&'static str, String)> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    if !matches!(ext, "js" | "mjs" | "cjs") {
        return None;
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, src, SourceType::mjs()).parse();
    if parsed.errors.is_empty() {
        return None;
    }
    if lexer_rejects(src) {
        return Some(("Error", IMPORT_ANALYSIS.to_string()));
    }
    let n = parsed.errors.len();
    let mut message = format!(
        "Parse failed with {n} error{}:\n",
        if n < 2 { "" } else { "s" }
    );
    let mut pos = None;
    for (i, e) in parsed.errors.iter().enumerate() {
        if i >= 5 {
            message.push_str("\n...");
            break;
        }
        let starts: Vec<usize> = e.labels.iter().flatten().map(|l| l.offset()).collect();
        if i == 0 {
            pos = starts.first().copied();
        }
        let frames: Vec<String> = starts
            .iter()
            .map(|&at| {
                let (line, column) = locate(src, at);
                code_frame(src, line, column)
            })
            .collect();
        message.push_str(&e.message);
        message.push('\n');
        message.push_str(&frames.join("\n"));
    }
    let url = module_url(path, root);
    let at = match pos {
        Some(at) => {
            let (line, column) = locate(src, at);
            format!("At file: {url}:{line}:{column}")
        }
        None => format!("At file: {url}"),
    };
    Some(("RolldownError", format!("Parse failure: {message}\n{at}")))
}

/// Whether the module `path` (source `src`) imports `spec` statically (`import … from`,
/// `export … from`): the module runner's form of the import is a `require()`, which fails as
/// vitest's import does when the specifier does not resolve (turbo-test#108), where a
/// `require()` the source wrote fails as Node's.
pub fn imports_statically(path: &Path, src: &str, spec: &str) -> bool {
    use oxc_ast::ast::Statement;
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::mjs());
    let parsed = Parser::new(&allocator, src, source_type).parse();
    parsed.program.body.iter().any(|st| match st {
        Statement::ImportDeclaration(d) => d.source.value == spec,
        Statement::ExportAllDeclaration(d) => d.source.value == spec,
        Statement::ExportNamedDeclaration(d) => d.source.as_ref().is_some_and(|s| s.value == spec),
        _ => false,
    })
}

const IMPORT_ANALYSIS: &str = "Failed to parse source for import analysis because the content contains invalid JS syntax. If you are using JSX, make sure to name the file with the .jsx or .tsx extension.";

/// vite's URL of a module: root-relative inside the root, `/@fs/<absolute>` outside it.
fn module_url(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => format!("/{}", rel.display()),
        Err(_) => format!("/@fs{}", path.display()),
    }
}

/// The 1-based line and 0-based column (in characters) of byte offset `at` of `src`.
fn locate(src: &str, at: usize) -> (usize, usize) {
    let before = &src[..at.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].chars().count())
}

/// rolldown's `getCodeFrame`: up to two lines before the error line and two after (trailing
/// blank lines dropped), numbered, leading tabs as two spaces, long lines cut, a caret under the
/// column.
fn code_frame(src: &str, line: usize, column: usize) -> String {
    const MIN_AFTER: usize = 10;
    const TRUNCATE: usize = 120;
    let lines: Vec<&str> = src.split('\n').collect();
    if line > lines.len() {
        return String::new();
    }
    // leading tabs as two spaces each
    let tabs = |s: &str| {
        let body = s.trim_start_matches('\t');
        format!("{}{body}", "  ".repeat(s.len() - body.len()))
    };
    let head: String = lines[line - 1].chars().take(column).collect();
    let max_len = (tabs(&head).chars().count() + MIN_AFTER + 3).max(TRUNCATE);
    let start = line.saturating_sub(3);
    let mut end = (line + 2).min(lines.len());
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let digits = end.to_string().len();
    (start..end)
        .map(|i| {
            let mut shown = tabs(lines[i]);
            if shown.chars().count() > max_len {
                shown = format!("{}...", shown.chars().take(max_len - 3).collect::<String>());
            }
            let number = format!("{:>digits$}", i + 1);
            if i + 1 == line {
                let indent = digits + 2 + tabs(&head).chars().count();
                format!("{number}: {shown}\n{}^", " ".repeat(indent))
            } else {
                format!("{number}: {shown}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether es-module-lexer rejects `src`: a `)` or `}` that closes nothing, a string that
/// reaches a line break or the end, a template or regular expression that is not closed, or
/// brackets / templates left open at the end. Strings, comments, templates and regular
/// expressions are skipped as the lexer skips them; a `/` starts a regular expression after an
/// operator, an opening bracket, a keyword that precedes an expression, or nothing.
fn lexer_rejects(src: &str) -> bool {
    let b = src.as_bytes();
    let mut i = 0;
    // open `(` / `{` / `${`: true marks a template substitution
    let mut open: Vec<bool> = Vec::new();
    // the last significant token: punctuator byte, or 'w' for a word / number / literal end
    let mut last: Option<u8> = None;
    let mut last_word = String::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
                i += 1;
                continue;
            }
            b'(' | b'{' => open.push(false),
            b')' | b'}' => match open.pop() {
                None => return true,
                Some(true) if c == b'}' => {
                    // back in the template after `${ … }`
                    match template(b, i + 1, &mut open) {
                        Some(next) => {
                            i = next;
                            last = Some(b'w');
                            continue;
                        }
                        None => return true,
                    }
                }
                Some(_) => {}
            },
            b'\'' | b'"' => {
                let mut j = i + 1;
                loop {
                    match b.get(j) {
                        None | Some(b'\n') | Some(b'\r') => return true,
                        Some(b'\\') => j += 2,
                        Some(&q) if q == c => break,
                        Some(_) => j += 1,
                    }
                }
                i = j + 1;
                last = Some(b'w');
                continue;
            }
            b'`' => match template(b, i + 1, &mut open) {
                Some(next) => {
                    i = next;
                    last = Some(b'w');
                    continue;
                }
                None => return true,
            },
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                continue;
            }
            b'/' if regex_allowed(last, &last_word) => {
                let mut j = i + 1;
                let mut class = false;
                loop {
                    match b.get(j) {
                        None | Some(b'\n') | Some(b'\r') => return true,
                        Some(b'\\') => j += 2,
                        Some(b'[') => {
                            class = true;
                            j += 1;
                        }
                        Some(b']') => {
                            class = false;
                            j += 1;
                        }
                        Some(b'/') if !class => break,
                        Some(_) => j += 1,
                    }
                }
                i = j + 1;
                last = Some(b'w');
                continue;
            }
            _ => {}
        }
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 {
            let s = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] >= 0x80)
            {
                i += 1;
            }
            last_word = src[s..i].to_string();
            last = Some(b'w');
            continue;
        }
        last = Some(c);
        last_word.clear();
        i += 1;
    }
    !open.is_empty()
}

/// Scan a template from `i` (just past the backtick or a substitution's `}`): the index past
/// its closing backtick, or past a `${` (pushed on `open`); None when the source ends first.
fn template(b: &[u8], mut i: usize, open: &mut Vec<bool>) -> Option<usize> {
    loop {
        match b.get(i)? {
            b'\\' => i += 2,
            b'`' => return Some(i + 1),
            b'$' if b.get(i + 1) == Some(&b'{') => {
                open.push(true);
                return Some(i + 2);
            }
            _ => i += 1,
        }
    }
}

/// Whether a `/` after `last` (a punctuator, `w` for a word / literal, or nothing) starts a
/// regular expression.
fn regex_allowed(last: Option<u8>, word: &str) -> bool {
    match last {
        None => true,
        Some(b'w') => matches!(
            word,
            "return"
                | "typeof"
                | "instanceof"
                | "in"
                | "of"
                | "new"
                | "delete"
                | "void"
                | "throw"
                | "case"
                | "do"
                | "else"
                | "yield"
                | "await"
        ),
        Some(b')' | b']') => false,
        Some(_) => true,
    }
}

#[cfg(test)]
#[path = "parse_error_tests.rs"]
mod tests;
