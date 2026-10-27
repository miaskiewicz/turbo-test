//! The errors vitest fails a module that does not parse or transform with (turbo-test#108,
//! turbo-test#113).
//!
//! vite transforms a `.ts` / `.tsx` / `.mts` / `.cts` / `.jsx` module with oxc first (its
//! `vite:oxc` plugin): a module oxc does not parse fails with `Transform failed with N error(s):`
//! and rolldown's diagnostics as ariadne renders them (coloured, whatever NO_COLOR says). A
//! `.js` / `.mjs` / `.cjs` module is parsed twice: the import analysis lexes it with
//! es-module-lexer (a source the lexer rejects — an unbalanced `)` / `}`, an unterminated string,
//! template or regular expression — fails with vite's "Failed to parse source for import analysis
//! …"), then rewrites its imports to vite's URLs, and the SSR transform parses that with
//! rolldown's oxc parser (any other syntax error is rolldown's `Parse failure: Parse failed with N
//! error(s): …` with a code frame of the rewritten code and the module's root-relative URL).
//! turbo-test asks for these only after its own transform or V8's compile failed: a source oxc
//! parses is left with V8's SyntaxError (an error only V8 reports, as in vitest; turbo-test#99).
//! Each error carries what vite's plugin container adds to it (the plugin, the module, the
//! position and vite's code frame), which vitest's reporters print under the message.

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// An error vite fails a module with: its name and message, and the details vitest keeps as
/// `__vitest_rollup_error__` (the plugin that threw, the module's id, the 1-based line and
/// 0-based column, vite's code frame).
#[derive(Debug, Clone, PartialEq)]
pub struct ViteError {
    pub name: &'static str,
    pub message: String,
    pub plugin: Option<&'static str>,
    pub id: String,
    pub loc: Option<(usize, usize)>,
    pub frame: Option<String>,
}

/// The error vitest fails the module `path` (source `src`, project root `root`) with when it
/// does not parse (a JavaScript module) or transform (TypeScript / JSX); `resolve` resolves an
/// import specifier of the module to a file (vite's import analysis rewrites the module's
/// imports to their URLs before the parse that fails).
pub fn vite_error(
    path: &Path,
    src: &str,
    root: &Path,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<ViteError> {
    match path.extension().and_then(|e| e.to_str())? {
        "js" | "mjs" | "cjs" => vite_parse_error(path, src, root, resolve),
        "ts" | "tsx" | "mts" | "cts" | "jsx" => vite_transform_error(path, src, root),
        _ => None,
    }
}

/// vite's error for a JavaScript module oxc does not parse.
fn vite_parse_error(
    path: &Path,
    src: &str,
    root: &Path,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<ViteError> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, src, SourceType::mjs()).parse();
    if parsed.errors.is_empty() {
        return None;
    }
    let lexed = lex(src);
    if let Some(at) = lexed.error {
        // vite's plugin container: the position and frame unless the source looks binary
        let shown = !src.contains('\u{fffd}');
        return Some(ViteError {
            name: "Error",
            message: IMPORT_ANALYSIS.to_string(),
            plugin: Some("vite:import-analysis"),
            id: path.display().to_string(),
            loc: shown.then(|| locate(src, at)),
            frame: shown.then(|| vite_code_frame(src, at)),
        });
    }
    // the code rolldown parses: the module with its imports rewritten to vite's URLs
    let code = rewrite_imports(src, &lexed.imports, root, resolve);
    let allocator = Allocator::default();
    let reparsed = Parser::new(&allocator, &code, SourceType::mjs()).parse();
    // only plain string literals were replaced, so it fails as the module does: the errors and
    // frames are the rewritten code's
    let code = code.as_str();
    let errors = &reparsed.errors;
    let n = errors.len();
    let mut message = format!(
        "Parse failed with {n} error{}:\n",
        if n < 2 { "" } else { "s" }
    );
    let mut pos = None;
    for (i, e) in errors.iter().enumerate() {
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
                let (line, column) = locate(code, at);
                code_frame(code, line, column)
            })
            .collect();
        message.push_str(&e.message);
        message.push('\n');
        message.push_str(&frames.join("\n"));
    }
    let url = module_url(path, root);
    let loc = pos.map(|at| locate(code, at));
    // (every oxc parse error has a label but "source too long", past 4 GiB)
    let mut at = format!("At file: {url}");
    if let Some((line, column)) = loc {
        at.push_str(&format!(":{line}:{column}"));
    }
    Some(ViteError {
        name: "RolldownError",
        message: format!("Parse failure: {message}\n{at}"),
        plugin: None,
        id: url,
        loc,
        frame: pos.map(|at| vite_code_frame(code, at)),
    })
}

/// vite's `vite:oxc` error for a TypeScript / JSX module oxc does not parse (or whose syntax its
/// semantic check rejects): `Transform failed with N error(s):` and each error as rolldown
/// renders it (ariadne, coloured; the file named relative to the working directory `root`).
fn vite_transform_error(path: &Path, src: &str, root: &Path) -> Option<ViteError> {
    let source_type = SourceType::from_path(path).ok()?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, src, source_type).parse();
    let errors = if parsed.errors.is_empty() {
        oxc_semantic::SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program)
            .errors
    } else {
        parsed.errors
    };
    if errors.is_empty() {
        return None;
    }
    let file = relative(path, root);
    let n = errors.len();
    let mut message = format!(
        "Transform failed with {n} error{}:\n",
        if n < 2 { "" } else { "s" }
    );
    for (i, e) in errors.iter().enumerate() {
        message.push('\n');
        if i >= 5 {
            message.push_str("...");
            break;
        }
        let labels: Vec<(usize, usize, String)> = e
            .labels
            .iter()
            .flatten()
            .map(|l| {
                (
                    l.offset(),
                    l.offset() + l.len(),
                    l.label().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let help = e.help.as_ref().map(|h| h.to_string());
        message.push_str(&rolldown_diagnostic(&file, src, &e.message, &labels, help));
    }
    Some(ViteError {
        name: "Error",
        message,
        plugin: Some("vite:oxc"),
        id: path.display().to_string(),
        loc: None,
        frame: None,
    })
}

/// rolldown's `PARSE_ERROR` diagnostic as it renders it in colour (`Diagnostic::to_color_string`):
/// an ariadne report of the error's labels in `file` (source `src`) with the help line; a label
/// whose line runs on for 300 characters either side of it in a source of 600 bytes or more is
/// named in the title instead (`is_context_too_long`).
fn rolldown_diagnostic(
    file: &str,
    src: &str,
    title: &str,
    labels: &[(usize, usize, String)],
    help: Option<String>,
) -> String {
    use ariadne::{Config, IndexType, Label, Report, ReportKind};
    let mut title = title.to_string();
    // (rolldown builds the report on an empty source's 0..0, so the header names the first label)
    let mut report =
        Report::build(ReportKind::Error, (String::new(), 0..0)).with_code("PARSE_ERROR");
    for (start, end, label) in labels {
        if context_too_long(src, *start, *end) {
            title.push_str(&format!("\n - {label} in {file} at {start}..{end}"));
        } else {
            report =
                report.with_label(Label::new((file.to_string(), *start..*end)).with_message(label));
        }
    }
    report.with_helps(help);
    let mut out = Vec::new();
    // (only the writer's errors come back, and writing to a Vec cannot fail)
    let _ = report
        .with_message(&title)
        .with_config(
            Config::default()
                .with_color(true)
                .with_index_type(IndexType::Byte)
                .with_severity_prefix(false),
        )
        .finish()
        .write_for_stdout(ariadne::sources([(file.to_string(), src)]), &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// rolldown's `is_context_too_long`: in a source of 600 bytes or more, whether the span
/// `start..end` has no line break within 300 characters before or after it.
fn context_too_long(src: &str, start: usize, end: usize) -> bool {
    if src.len() < 600 {
        return false;
    }
    let (Some(before), Some(after)) = (src.get(..start), src.get(end..)) else {
        return true;
    };
    let bounded = |chars: &mut dyn Iterator<Item = char>| {
        let mut n = 0;
        for c in chars {
            if c == '\n' {
                return true;
            }
            n += 1;
            if n > 300 {
                break;
            }
        }
        n < 300
    };
    !bounded(&mut after.chars()) || !bounded(&mut before.chars().rev())
}

/// `path` relative to `base`, with `/` separators (rolldown's `stabilize_path`).
fn relative(path: &Path, base: &Path) -> String {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); b.len() - common];
    parts.extend(
        p[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
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

/// vite's `generateCodeFrame(source, pos)` (what its plugin container and SSR transform attach
/// as `frame`): two lines either side of the one holding byte offset `at`, numbered in a column
/// of at least three, `|  ` and the line (a line longer than 120 characters cut around the
/// position with `...`), a `^` under the position.
fn vite_code_frame(src: &str, at: usize) -> String {
    const MAX: usize = 120;
    let (line, column) = locate(src, at);
    let width = 3.max(line.to_string().len() + 1);
    let lines: Vec<&str> = src
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let i = line - 1;
    let mut out = Vec::new();
    let first = i.saturating_sub(2);
    for (j, text) in lines.iter().enumerate().take(i + 3).skip(first) {
        let chars: Vec<char> = text.chars().collect();
        let mut shown = text.to_string();
        let mut under = column;
        if chars.len() > MAX {
            let from = if j == i {
                column.saturating_sub(MAX / 2)
            } else {
                0
            };
            if j == i {
                under = column - from + if from > 0 { 3 } else { 0 };
            }
            let prefix = if from > 0 { "..." } else { "" };
            let suffix = if chars.len() - from > MAX { "..." } else { "" };
            let take = MAX - prefix.len() - suffix.len();
            let body: String = chars[from..].iter().take(take).collect();
            shown = format!("{prefix}{body}{suffix}");
        }
        let number = (j + 1).to_string();
        out.push(format!(
            "{number}{}|  {shown}",
            " ".repeat(width - number.len())
        ));
        if j == i {
            out.push(format!("{}|  {}^", " ".repeat(width), " ".repeat(under)));
        }
    }
    out.join("\n")
}

/// What es-module-lexer makes of a source: the byte offset it fails at (`e.idx`: a `)` or `}`
/// that closes nothing; the line break or end a string or regular expression reaches; the end
/// an open template reaches; 0 for brackets left open at the end), and the spans (quotes
/// included) of the string specifiers of its imports (`import … from '…'`, `import '…'`,
/// `export … from '…'`, `import('…')`).
struct Lexed {
    error: Option<usize>,
    imports: Vec<std::ops::Range<usize>>,
}

/// Lex `src` as es-module-lexer does (see [`Lexed`]): strings, comments, templates and regular
/// expressions are skipped as the lexer skips them; a `/` starts a regular expression after an
/// operator, an opening bracket, a keyword that precedes an expression, or nothing.
fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut i = 0;
    let mut imports = Vec::new();
    let failed = |at: usize, imports: Vec<std::ops::Range<usize>>| Lexed {
        error: Some(at),
        imports,
    };
    // open `(` / `{` / `${`: true marks a template substitution
    let mut open: Vec<bool> = Vec::new();
    // the last significant token: punctuator byte, or 'w' for a word / number / literal end
    let mut last: Option<u8> = None;
    let mut last_word = String::new();
    // the last token is the `(` of `import(`
    let mut import_call = false;
    while i < b.len() {
        let c = b[i];
        let after_import_call = std::mem::take(&mut import_call);
        match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
                import_call = after_import_call;
                i += 1;
                continue;
            }
            b'(' | b'{' => {
                import_call = c == b'(' && last == Some(b'w') && last_word == "import";
                open.push(false);
            }
            b')' | b'}' => match open.pop() {
                None => return failed(i, imports),
                Some(true) if c == b'}' => {
                    // back in the template after `${ … }`
                    match template(b, i + 1, &mut open) {
                        Some(next) => {
                            i = next;
                            last = Some(b'w');
                            last_word.clear();
                            continue;
                        }
                        None => return failed(b.len(), imports),
                    }
                }
                Some(_) => {}
            },
            b'\'' | b'"' => {
                let mut j = i + 1;
                loop {
                    match b.get(j) {
                        None => return failed(b.len(), imports),
                        Some(b'\n') | Some(b'\r') => return failed(j, imports),
                        Some(b'\\') => j += 2,
                        Some(&q) if q == c => break,
                        Some(_) => j += 1,
                    }
                }
                let specifier = after_import_call
                    || (last == Some(b'w') && matches!(last_word.as_str(), "from" | "import"));
                if specifier {
                    imports.push(i..j + 1);
                }
                i = j + 1;
                last = Some(b'w');
                last_word.clear();
                continue;
            }
            b'`' => match template(b, i + 1, &mut open) {
                Some(next) => {
                    i = next;
                    last = Some(b'w');
                    last_word.clear();
                    continue;
                }
                None => return failed(b.len(), imports),
            },
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                import_call = after_import_call;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                import_call = after_import_call;
                continue;
            }
            b'/' if regex_allowed(last, &last_word) => {
                let mut j = i + 1;
                let mut class = false;
                loop {
                    match b.get(j) {
                        None => return failed(b.len(), imports),
                        Some(b'\n') | Some(b'\r') => return failed(j, imports),
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
                last_word.clear();
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
    Lexed {
        error: (!open.is_empty()).then_some(0),
        imports,
    }
}

/// Whether es-module-lexer rejects `src`.
#[cfg(test)]
fn lexer_rejects(src: &str) -> bool {
    lex(src).error.is_some()
}

/// `src` with the import specifiers at `imports` rewritten as vite's import analysis rewrites
/// them: a specifier that resolves (`resolve`) to a file becomes its URL in double quotes
/// (root-relative inside `root`, `/@fs/<path>` outside it); a `node:` built-in and a specifier
/// that does not resolve are left as written.
fn rewrite_imports(
    src: &str,
    imports: &[std::ops::Range<usize>],
    root: &Path,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for span in imports {
        let spec = &src[span.start + 1..span.end - 1];
        if spec.starts_with("node:") || spec.contains('\\') {
            continue;
        }
        let Some(file) = resolve(spec) else {
            continue;
        };
        out.push_str(&src[at..span.start]);
        out.push('"');
        out.push_str(&module_url(&file, &root));
        out.push('"');
        at = span.end;
    }
    out.push_str(&src[at..]);
    out
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
