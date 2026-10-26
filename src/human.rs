//! vitest 4.1.12's human reporters, as they print to a file or pipe without colours (`NO_COLOR`,
//! or under an AI agent): `default`, `verbose` and `agent` (turbo-test#94).
//!
//! stdout: the ` RUN  v4.1.12 <root>` banner, each file's console output as
//! `stdout | <file> > <task>` blocks, then the reporter's lines — `default`: a module line per
//! file (` ✓ a.test.ts (2 tests) 3ms`) and, for a failed file, a line per test indented by its
//! `describe` depth; `verbose`: a line per test (` ✓ a.test.ts > d > t 1ms`, a failure's messages
//! under it); `agent`: only failed files and their failed tests, and only failed tasks' console
//! output — and the footer (Snapshots, Test Files, Tests, Start at, Duration with its breakdown).
//! stderr: the `stderr | …` blocks and the Failed Suites / Failed Tests sections, each error
//! printed with the code frame of its nearest frame and numbered dividers.

use std::path::{Path, PathBuf};

use crate::cli::FileResult;
use crate::failure::{ErrorInfo, Frame};
use crate::runner::TestCase;

/// Which human reporter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Default,
    Verbose,
    Agent,
}

/// The run as the footer reports it.
pub struct Run<'a> {
    /// The project root (the banner, relative frame paths).
    pub root: &'a Path,
    /// How each result's file is named (relative to the root).
    pub names: &'a [PathBuf],
    /// Wall time of the run, ms.
    pub wall_ms: f64,
    /// When the run started, as the local `HH:MM:SS`.
    pub start: String,
    /// The Test Files and Tests lines (cli::footer's).
    pub test_files: String,
    pub tests: String,
}

/// `(stdout, stderr)` of `kind` for the run.
pub fn render(kind: Kind, res: &[FileResult], run: &Run) -> (String, String) {
    let (mut out, mut err) = (String::new(), String::new());
    out.push_str(&format!("\n RUN  v4.1.12 {}\n\n", run.root.display()));
    for r in res {
        let name = run.names[r.idx].to_string_lossy().to_string();
        let failed_tasks: Vec<String> = r
            .tests
            .iter()
            .filter(|t| t.status == "failed")
            .map(test_path)
            .chain(r.suite_errors.iter().map(|s| s.name.clone()))
            .collect();
        for (stream, task, content) in &r.logs {
            if kind == Kind::Agent && !failed_tasks.contains(task) {
                continue;
            }
            let header = if task.is_empty() {
                name.clone()
            } else {
                format!("{name} > {task}")
            };
            let block = format!("{stream} | {header}\n{content}\n");
            if stream == "stderr" {
                err.push_str(&block);
            } else {
                out.push_str(&block);
            }
        }
        match kind {
            Kind::Verbose => {
                for t in &r.tests {
                    out.push_str(&format!(
                        " {} {name} > {}{}\n",
                        symbol(t),
                        test_path(t),
                        suffix(t)
                    ));
                    if t.status == "failed" {
                        for e in errors(t) {
                            out.push_str(&format!("   → {}\n", e.message));
                        }
                    }
                }
            }
            Kind::Default | Kind::Agent => {
                if kind == Kind::Agent && !r.failed_file {
                    continue;
                }
                out.push_str(&module_line(r, &name));
                for t in &r.tests {
                    let failed = t.status == "failed";
                    if !failed && (kind == Kind::Agent || !r.failed_file) {
                        continue;
                    }
                    let pad = "  ".repeat(t.ancestors.len() + 1);
                    out.push_str(&format!(" {pad}{} {}{}\n", symbol(t), t.title, suffix(t)));
                }
            }
        }
    }
    err.push_str(&errors_summary(res, run));
    out.push('\n');
    for line in snapshot_lines(res) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&format!(" Test Files  {}\n", run.test_files));
    out.push_str(&format!("      Tests  {}\n", run.tests));
    out.push_str(&format!("   Start at  {}\n", run.start));
    let (tests, setup, files) = res.iter().fold((0.0, 0.0, 0.0), |(t, s, f), r| {
        let ran: f64 = r.tests.iter().map(|t| t.duration_ms).sum();
        (t + ran, s + r.setup_us / 1000.0, f + r.dur_ms)
    });
    out.push_str(&format!(
        "   Duration  {} (transform 0ms, setup 0ms, import {}, tests {}, environment {})\n\n",
        format_time(run.wall_ms),
        format_time((files - tests - setup).max(0.0)),
        format_time(tests),
        format_time(setup),
    ));
    (out, err)
}

/// vitest's `formatTime`.
pub fn format_time(ms: f64) -> String {
    if ms > 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else {
        format!("{}ms", ms.round())
    }
}

/// A test's name path below its file (`describe > it`).
fn test_path(t: &TestCase) -> String {
    let mut parts = t.ancestors.clone();
    parts.push(t.title.clone());
    parts.join(" > ")
}

/// vitest's state symbol for a test.
fn symbol(t: &TestCase) -> &'static str {
    match t.status.as_str() {
        "passed" => "✓",
        "failed" => "×",
        "todo" => "□",
        "skipped" => "↓",
        _ => "·",
    }
}

/// ` 3ms` for a test that ran (vitest prints the rounded duration of a test with a result).
fn suffix(t: &TestCase) -> String {
    if (t.status == "passed" || t.status == "failed") && !t.not_run {
        format!(" {}ms", t.duration_ms.round())
    } else {
        String::new()
    }
}

/// A failed test's errors (one from its message when the runtime recorded no error object).
fn errors(t: &TestCase) -> Vec<ErrorInfo> {
    if t.errors.is_empty() {
        vec![ErrorInfo {
            name: "Error".into(),
            message: t.message.clone(),
            ..Default::default()
        }]
    } else {
        t.errors.clone()
    }
}

/// ` ❯ a.test.ts (7 tests | 3 failed | 1 skipped | 1 todo) 118ms`.
fn module_line(r: &FileResult, name: &str) -> String {
    let n = r.tests.len();
    let count = |s: &str| r.tests.iter().filter(|t| t.status == s).count();
    let mut state = format!("{n} test{}", if n > 1 { "s" } else { "" });
    for (k, label) in [
        ("failed", "failed"),
        ("skipped", "skipped"),
        ("todo", "todo"),
    ] {
        if count(k) > 0 {
            state.push_str(&format!(" | {} {label}", count(k)));
        }
    }
    let ran = r
        .tests
        .iter()
        .any(|t| t.status == "passed" || t.status == "failed");
    let sym = if r.failed_file {
        "❯"
    } else if !ran && n > 0 {
        "↓"
    } else {
        "✓"
    };
    let dur = if r.load_error {
        String::new()
    } else {
        format!(" {}ms", r.dur_ms.round())
    };
    format!(" {sym} {name} ({state}){dur}\n")
}

/// The footer's Snapshots lines (vitest's renderSnapshotSummary).
fn snapshot_lines(res: &[FileResult]) -> Vec<String> {
    let (mut added, mut unmatched, mut updated, mut unchecked) = (0, 0, 0, 0);
    for s in res.iter().filter_map(|r| r.snapshot.as_ref()) {
        added += s.added;
        unmatched += s.unmatched;
        updated += s.updated;
        unchecked += s.unchecked;
    }
    let mut items = Vec::new();
    if added > 0 {
        items.push(format!("{added} written"));
    }
    if unmatched > 0 {
        items.push(format!("{unmatched} failed"));
    }
    if updated > 0 {
        items.push(format!("{updated} updated "));
    }
    if unchecked > 0 {
        items.push(format!("{unchecked} obsolete"));
    }
    let mut lines: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{:>11}  {s}", if i == 0 { "Snapshots" } else { "" }))
        .collect();
    if lines.len() > 1 {
        lines.push(String::new());
    }
    lines
}

/// One error to report: the name it is reported under (` FAIL  <name>`) and the error.
struct Reported {
    name: String,
    error: ErrorInfo,
}

/// The Failed Suites / Failed Tests sections (stderr).
fn errors_summary(res: &[FileResult], run: &Run) -> String {
    let mut suites: Vec<Reported> = Vec::new();
    let mut tests: Vec<Reported> = Vec::new();
    for r in res {
        let name = run.names[r.idx].to_string_lossy().to_string();
        if r.failed_file && r.tests.is_empty() && r.suite_errors.is_empty() {
            let error = r.load_error_info.clone().unwrap_or_else(|| ErrorInfo {
                name: "Error".into(),
                message: crate::cli::file_message(&r.error_msg),
                ..Default::default()
            });
            suites.push(Reported {
                name: format!("{name} [ {name} ]"),
                error,
            });
        }
        for s in &r.suite_errors {
            for e in &s.errors {
                suites.push(Reported {
                    name: format!("{name} > {}", s.name),
                    error: e.clone(),
                });
            }
        }
        for t in r.tests.iter().filter(|t| t.status == "failed") {
            for e in errors(t) {
                tests.push(Reported {
                    name: format!("{name} > {}", test_path(t)),
                    error: e,
                });
            }
        }
    }
    let total = suites.len() + tests.len();
    let mut out = String::new();
    let mut n = 0;
    for (title, list) in [("Failed Suites", &suites), ("Failed Tests", &tests)] {
        if list.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "\n{}\n\n",
            divider(&format!(" {title} {} ", list.len()), None)
        ));
        for rep in list.iter() {
            n += 1;
            out.push_str(&format!(" FAIL  {}\n", rep.name));
            out.push_str(&print_error(&rep.error, run.root, true, ""));
            out.push_str(&format!(
                "{}\n\n",
                divider(&format!("[{n}/{total}]"), Some(1))
            ));
        }
    }
    out
}

/// vitest's `divider` in a 30-column non-terminal: the text centred in `⎯`, or with `right`
/// `⎯` after it.
fn divider(text: &str, right: Option<usize>) -> String {
    let cols = 30usize;
    let len = text.chars().count();
    let (left, right) = match right {
        Some(r) => (cols.saturating_sub(len + r), r),
        None => {
            let left = cols.saturating_sub(len) / 2;
            (left, cols.saturating_sub(len + left))
        }
    };
    format!("{}{text}{}", "⎯".repeat(left), "⎯".repeat(right))
}

/// vitest's `printErrorInner` into a string: `Name: message`, the diff, the frames outside
/// node_modules with the code frame after the nearest one (`code_frame`), a blank line, the
/// cause.
pub fn print_error(e: &ErrorInfo, root: &Path, code_frame: bool, prefix: &str) -> String {
    let mut out = format!("{prefix}{}: {}\n", e.name, e.message);
    if !e.diff.is_empty() {
        out.push_str(&format!("\n{}\n\n", e.diff));
    }
    let frames: Vec<&Frame> = e
        .frames
        .iter()
        .filter(|f| !f.file.components().any(|c| c.as_os_str() == "node_modules"))
        .collect();
    let nearest = frames.iter().position(|f| f.file.is_file());
    for (i, f) in frames.iter().enumerate() {
        let path = f.file.strip_prefix(root).unwrap_or(&f.file);
        let at = format!("{}:{}:{}", path.display(), f.line, f.col);
        match f.method.as_str() {
            "" => out.push_str(&format!(" ❯ {at}\n")),
            m => out.push_str(&format!(" ❯ {m} {at}\n")),
        }
        if code_frame && Some(i) == nearest {
            if let Ok(source) = std::fs::read_to_string(&f.file) {
                out.push_str(&code_frame_of(&source, f.line, f.col));
                out.push('\n');
            }
        }
    }
    if !frames.is_empty() {
        out.push('\n');
    }
    if let Some(cause) = &e.cause {
        out.push_str(&print_error(cause, root, false, "Caused by: "));
    }
    out
}

/// vitest's `generateCodeFrame(source, 4, { line, column })` for an 80-column output: two lines
/// either side, each truncated to 71 characters with `…`, and a caret under the column.
pub fn code_frame_of(source: &str, line: u32, column: u32) -> String {
    let lines: Vec<&str> = source
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let i = (line as usize)
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let lo = i.saturating_sub(2);
    let hi = (i + 2).min(lines.len() - 1);
    let no = |n: Option<usize>| match n {
        Some(n) => format!("{n:>3}|"),
        None => "   |".to_string(),
    };
    let mut out = Vec::new();
    for (j, text) in lines.iter().enumerate().take(hi + 1).skip(lo) {
        if text.starts_with("//# sourceMappingURL") {
            continue;
        }
        if text.chars().count() > 200 {
            return String::new();
        }
        let text = text.replace('\t', " ");
        let truncated = if text.chars().count() <= 71 {
            text
        } else {
            format!("{}…", text.chars().take(70).collect::<String>())
        };
        let truncated = truncated.trim_end();
        out.push(if truncated.is_empty() {
            no(Some(j + 1))
        } else {
            format!("{} {truncated}", no(Some(j + 1)))
        });
        if j == i {
            out.push(format!("{}{}^", no(None), " ".repeat(column as usize)));
        }
    }
    out.iter()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "human_tests.rs"]
mod tests;
