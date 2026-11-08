//! vitest 4.1.12's human reporters as they print to a file or pipe: `default`, `verbose`,
//! `agent` (turbo-test#94) and `dot` (turbo-test#100), coloured as vitest colours them (tinyrainbow, `crate::colors`) unless
//! `NO_COLOR` / `TERM=dumb` / an AI agent turns colours off; code frames are syntax-highlighted
//! (`crate::highlight`).
//!
//! stdout: the ` RUN  v4.1.12 <root>` banner, each file's console output as
//! `stdout | <file> > <task>` blocks, then the reporter's lines — `default`: a module line per
//! file (` ✓ a.test.ts (2 tests) 3ms`) and, for a failed file, a line per test indented by its
//! `describe` depth; `verbose`: a line per test (` ✓ a.test.ts > d > t 1ms`, a failure's messages
//! under it); `agent`: only failed files and their failed tests, and only failed tasks' console
//! output; `dot`: one character per test (`·` passed, `x` failed, `-` skipped or todo) and an
//! empty line — and the footer (Snapshots, Test Files, Tests, Start at, Duration with its breakdown).
//! stderr: the `stderr | …` blocks and the Failed Suites / Failed Tests sections, each error
//! printed with the code frame of its nearest frame and numbered dividers.

use std::path::{Path, PathBuf};

use crate::cli::FileResult;
use crate::colors::{strip, Colors};
use crate::failure::{ErrorInfo, Frame};
use crate::runner::{Annotation, TestCase};

/// Which human reporter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Default,
    Verbose,
    Agent,
    Dot,
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
    /// `-u` / `--update`: obsolete snapshots were removed.
    pub did_update: bool,
    /// vitest's colours (off for the agent reporter).
    pub colors: Colors,
}

/// vitest's default `slowTestThreshold`, ms: a slower passing test gets a line of its own.
const SLOW_MS: f64 = 300.0;

/// `(stdout, stderr)` of `kind` for the run.
pub fn render(kind: Kind, res: &[FileResult], run: &Run) -> (String, String) {
    let c = &run.colors;
    let (mut out, mut err) = (String::new(), String::new());
    // printBanner: withLabel('cyan', 'RUN', `v${version} `) + c.gray(root)
    out.push_str(&format!(
        "\n{} {}{}\n\n",
        c.bold(&c.black(&c.bg_cyan(" RUN "))),
        c.cyan("v4.1.12 "),
        c.gray(&run.root.display().to_string())
    ));
    for r in res {
        let name = run.names[r.idx].to_string_lossy().to_string();
        console_logs(c, kind, r, &name, &mut out, &mut err);
        match kind {
            Kind::Dot => dot_tests(c, r, &mut out),
            Kind::Verbose => verbose_tests(c, r, &name, &mut out),
            Kind::Default | Kind::Agent => default_tests(c, kind, r, &name, &mut out),
        }
    }
    err.push_str(&errors_summary(res, run, kind == Kind::Verbose));
    if kind == Kind::Dot {
        // DotReporter.onTestRunEnd off a TTY: logger.log()
        out.push('\n');
    }
    summary(c, res, run, &mut out);
    (out, err)
}

/// A file's console output, each block under its task's header (onUserConsoleLog).
fn console_logs(
    c: &Colors,
    kind: Kind,
    r: &FileResult,
    name: &str,
    out: &mut String,
    err: &mut String,
) {
    let sep = c.dim(" > ");
    let logs: Vec<&(String, String, String)> = if kind == Kind::Agent {
        agent_logs(r)
    } else {
        r.logs.iter().collect()
    };
    for (stream, task, content) in logs {
        // onUserConsoleLog: c.gray(type + c.dim(` | ${getFullName(task, separator)}\n`))
        let header = if task.is_empty() {
            name.to_string()
        } else {
            format!(
                "{name}{sep}{}",
                task.split(" > ").collect::<Vec<_>>().join(&sep)
            )
        };
        let block = format!(
            "{}{content}\n",
            c.gray(&format!("{stream}{}", c.dim(&format!(" | {header}\n"))))
        );
        if stream == "stderr" {
            err.push_str(&block);
        } else {
            out.push_str(&block);
        }
    }
}

/// DotReporter.onTestCaseResult off a TTY: formatTests([state]) for each test, an empty yellow
/// (pending) run before its icon.
fn dot_tests(c: &Colors, r: &FileResult, out: &mut String) {
    for t in &r.tests {
        out.push_str(&c.yellow(""));
        out.push_str(&match t.status.as_str() {
            "passed" => c.green("·"),
            "failed" => c.red("x"),
            _ => c.dim(&c.gray("-")),
        });
    }
}

/// The verbose reporter's line per test, its errors' messages and its annotations.
fn verbose_tests(c: &Colors, r: &FileResult, name: &str, out: &mut String) {
    let sep = c.dim(" > ");
    for t in &r.tests {
        let path = test_path(t).split(" > ").collect::<Vec<_>>().join(&sep);
        out.push_str(&format!(
            " {} {name}{sep}{path}{}\n",
            symbol(c, t),
            suffix(c, t)
        ));
        if t.status == "failed" {
            for e in errors(t) {
                let message = if e.no_message {
                    "undefined"
                } else {
                    &e.message
                };
                out.push_str(&c.red(&format!("   → {message}")));
                out.push('\n');
            }
        }
        if !t.annotations.is_empty() {
            out.push('\n');
            out.push_str(&annotation_lines(c, &t.annotations, name, 3));
            out.push('\n');
        }
    }
}

/// The default (and agent) reporter's module line and DefaultReporter.printTestCase (the agent
/// reporter prints only failed files' failed tests).
fn default_tests(c: &Colors, kind: Kind, r: &FileResult, name: &str, out: &mut String) {
    if kind == Kind::Agent && !r.failed_file {
        return;
    }
    out.push_str(&module_line(c, r, name));
    for t in &r.tests {
        let pad = "  ".repeat(t.ancestors.len() + 1);
        let (title, suffix) = (&t.title, suffix(c, t));
        let line = if t.status == "failed" {
            c.red(&format!(" {pad}{} {title}", c.red("×"))) + &suffix
        } else if kind == Kind::Agent {
            continue;
        } else if t.duration_ms > SLOW_MS {
            format!(" {pad}{} {title} {suffix}", c.yellow(&c.dim("✓")))
        } else if r.failed_file {
            format!(" {pad}{} {title}{suffix}", symbol(c, t))
        } else {
            continue;
        };
        out.push_str(&line);
        out.push('\n');
    }
}

/// reportTestSummary: the Snapshots, Test Files, Tests, Start at and Duration lines.
fn summary(c: &Colors, res: &[FileResult], run: &Run, out: &mut String) {
    // reportTestSummary: padSummaryTitle(title) = c.dim(`${title.padStart(11)} `), then a space
    let title = |t: &str| c.dim(&format!("{t:>11} "));
    out.push('\n');
    for (i, line) in snapshot_lines(res, run).iter().enumerate() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let t = if i == 0 { "Snapshots" } else { "" };
            out.push_str(&format!("{} {line}\n", title(t)));
        }
    }
    out.push_str(&format!("{} {}\n", title("Test Files"), run.test_files));
    out.push_str(&format!("{} {}\n", title("Tests"), run.tests));
    out.push_str(&format!("{} {}\n", title("Start at"), run.start));
    let (tests, setup, files) = res.iter().fold((0.0, 0.0, 0.0), |(t, s, f), r| {
        let ran: f64 = r.tests.iter().map(|t| t.duration_ms).sum();
        (t + ran, s + r.setup_us / 1000.0, f + r.dur_ms + r.waited_ms)
    });
    out.push_str(&format!(
        "{} {}{}\n\n",
        title("Duration"),
        format_time(run.wall_ms),
        c.dim(&format!(
            " (transform 0ms, setup 0ms, import {}, tests {}, environment {})",
            format_time((files - tests - setup).max(0.0)),
            format_time(tests),
            format_time(setup),
        ))
    ));
}

/// The console output the agent reporter prints (vitest's `silent: 'passed-only'`): only that of
/// failed tasks — the file, a suite holding a failed test or whose hook failed, a failed test —
/// each task's output when the task finishes (tests, then their suites inside out, the file last).
fn agent_logs(r: &FileResult) -> Vec<&(String, String, String)> {
    let paths: Vec<String> = r.tests.iter().map(test_path).collect();
    let within = |p: &str, task: &str| p == task || p.starts_with(&format!("{task} > "));
    let failed = |task: &str| {
        task.is_empty() && r.failed_file
            || r.tests
                .iter()
                .zip(&paths)
                .any(|(t, p)| t.status == "failed" && within(p, task))
            || r.suite_errors
                .iter()
                .any(|s| task == s.name || task.ends_with(&format!(" > {}", s.name)))
    };
    // the order tasks finish in: each test, then each suite it closes (innermost first)
    let mut order: Vec<String> = Vec::new();
    for (i, t) in r.tests.iter().enumerate() {
        order.push(paths[i].clone());
        let next = r.tests.get(i + 1).map_or(&[][..], |n| &n.ancestors[..]);
        for depth in (0..t.ancestors.len()).rev() {
            if next.get(..=depth) != Some(&t.ancestors[..=depth]) {
                order.push(t.ancestors[..=depth].join(" > "));
            }
        }
    }
    let rank = |task: &str| {
        if task.is_empty() {
            usize::MAX
        } else {
            order
                .iter()
                .position(|o| o == task)
                .unwrap_or(usize::MAX - 1)
        }
    };
    let mut logs: Vec<&(String, String, String)> =
        r.logs.iter().filter(|(_, task, _)| failed(task)).collect();
    logs.sort_by_key(|(_, task, _)| rank(task));
    logs
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

/// vitest's state symbol for a test (`getStateSymbol`).
fn symbol(c: &Colors, t: &TestCase) -> String {
    match t.status.as_str() {
        "passed" => c.green("✓"),
        "failed" => c.red("×"),
        "todo" => c.dim(&c.gray("□")),
        "skipped" => c.dim(&c.gray("↓")),
        _ => c.gray("·"),
    }
}

/// vitest's getDurationPrefix: ` 3ms`, yellow over the slow-test threshold, green otherwise.
fn duration(c: &Colors, ms: f64) -> String {
    let d = ms.round();
    let text = format!(" {d}{}", c.dim("ms"));
    if d > SLOW_MS {
        c.yellow(&text)
    } else {
        c.green(&text)
    }
}

/// vitest's getTestCaseSuffix: the duration of a test that ran (one with a result: a test
/// skipped at run time has a start, turbo-test#96), then ` (retry x2)` for a retried one, ` (repeat x2)` for a repeated one and
/// ` [note]` for one skipped with a note.
fn suffix(c: &Colors, t: &TestCase) -> String {
    let mut s = String::new();
    let ran = t.status == "passed" || t.status == "failed" || t.start_ms > 0.0;
    if ran && !t.not_run {
        s.push_str(&duration(c, t.duration_ms));
    }
    if t.retry_count > 0 {
        s.push_str(&c.yellow(&format!(" (retry x{})", t.retry_count)));
    }
    if t.repeat_count > 0 {
        s.push_str(&c.yellow(&format!(" (repeat x{})", t.repeat_count)));
    }
    if t.status == "skipped" && !t.note.is_empty() {
        s.push_str(&c.dim(&c.gray(&format!(" [{}]", t.note))));
    }
    s
}

/// vitest's printAnnotations (turbo-test#96): the annotations grouped by where they were made
/// and their type (` ❯ a.test.ts:3:9 notice`), each message under its group (`  ↳ message`),
/// every line indented by `pad` spaces; `file` is the test file's name.
fn annotation_lines(c: &Colors, annotations: &[Annotation], file: &str, pad: usize) -> String {
    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for a in annotations {
        // `${c.gray(`${file}:${line}:${column}`)} ${c.bold(type)}`, or c.bold(type)
        let group = if a.line > 0 {
            format!(
                "{} {}",
                c.gray(&format!("{file}:{}:{}", a.line, a.col)),
                c.bold(&a.kind)
            )
        } else {
            c.bold(&a.kind)
        };
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, messages)) => messages.push(&a.message),
            None => groups.push((group, vec![&a.message])),
        }
    }
    let pad = " ".repeat(pad);
    let mut out = String::new();
    for (group, messages) in groups {
        out.push_str(&format!("{pad}{} {group}\n", c.blue("❯")));
        for m in messages {
            out.push_str(&format!("{pad}  {} {m}\n", c.blue("↳")));
        }
    }
    out
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

/// ` ❯ a.test.ts (7 tests | 3 failed | 1 skipped | 1 todo) 118ms` (vitest's getModuleLog).
fn module_line(c: &Colors, r: &FileResult, name: &str) -> String {
    let n = r.tests.len();
    let count = |s: &str| r.tests.iter().filter(|t| t.status == s).count();
    let mut state = c.dim(&format!("{n} test{}", if n > 1 { "s" } else { "" }));
    type Paint = fn(&Colors, &str) -> String;
    let parts: [(&str, Paint); 3] = [
        ("failed", Colors::red),
        ("skipped", Colors::yellow),
        ("todo", Colors::gray),
    ];
    for (k, color) in parts {
        if count(k) > 0 {
            state.push_str(&c.dim(" | "));
            state.push_str(&color(c, &format!("{} {k}", count(k))));
        }
    }
    let ran = r
        .tests
        .iter()
        .any(|t| t.status == "passed" || t.status == "failed");
    let sym = if r.failed_file {
        c.red("❯")
    } else if !ran && n > 0 {
        c.dim(&c.gray("↓"))
    } else {
        c.green("✓")
    };
    let dur = if r.load_error {
        String::new()
    } else {
        duration(c, r.dur_ms + r.waited_ms)
    };
    format!(" {sym} {name} {}{state}{}{dur}\n", c.dim("("), c.dim(")"))
}

/// The footer's Snapshots lines (vitest's renderSnapshotSummary over the files' results added
/// up): written, failed, updated, files removed / obsolete, then the obsolete (or, under `-u`,
/// removed) snapshots with `↳ ./file` and `  · key` lines per file; an empty line after more
/// than one (the summary prints a blank line there).
fn snapshot_lines(res: &[FileResult], run: &Run) -> Vec<String> {
    let c = &run.colors;
    let (mut added, mut unmatched, mut updated, mut unchecked, mut files_removed) = (0, 0, 0, 0, 0);
    let mut keys = Vec::new();
    for r in res {
        let Some(s) = &r.snapshot else { continue };
        added += s.added;
        unmatched += s.unmatched;
        updated += s.updated;
        unchecked += s.unchecked;
        files_removed += u32::from(s.file_deleted);
        if !s.unchecked_keys.is_empty() {
            let name = run.names[r.idx].to_string_lossy().replace('\\', "/");
            keys.push(format!("{} {}", c.gray("↳"), test_file_path(c, &name)));
            keys.extend(
                s.unchecked_keys
                    .iter()
                    .map(|k| format!("  {} {k}", c.gray("·"))),
            );
        }
    }
    // `N files removed ` / `N obsolete` …: removed (green) under -u, obsolete (yellow) otherwise
    let gone = |text: String| {
        if run.did_update {
            c.bold(&c.green(&text.replace("{}", "removed")))
        } else {
            c.bold(&c.yellow(&text.replace("{}", "obsolete")))
        }
    };
    let mut items = Vec::new();
    if added > 0 {
        items.push(c.bold(&c.green(&format!("{added} written"))));
    }
    if unmatched > 0 {
        items.push(c.bold(&c.red(&format!("{unmatched} failed"))));
    }
    if updated > 0 {
        items.push(c.bold(&c.green(&format!("{updated} updated "))));
    }
    if files_removed > 0 {
        items.push(gone(format!("{files_removed} files {{}} ")));
    }
    if unchecked > 0 {
        items.push(gone(format!("{unchecked} {{}}")));
        items.extend(keys);
    }
    if items.len() > 1 {
        items.push(String::new());
    }
    items
}

/// vitest's formatTestPath for a root-relative file: `dir/` dim, the base name bold, a
/// `.test.ts` / `.spec.mjs` … extension dim.
fn test_file_path(c: &Colors, name: &str) -> String {
    let p = Path::new(name);
    let dir = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_string_lossy().to_string(),
        _ => ".".to_string(),
    };
    let file = p
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    // (\.(spec|test)\.[cm]?[tj]sx?)$
    let ext = ["spec", "test"]
        .iter()
        .find_map(|kind| {
            let at = file.rfind(&format!(".{kind}."))?;
            let rest = &file[at + kind.len() + 2..];
            let lang = rest.strip_prefix(['c', 'm']).unwrap_or(rest);
            matches!(lang, "ts" | "js" | "tsx" | "jsx").then(|| file[at..].to_string())
        })
        .unwrap_or_default();
    let base = &file[..file.len() - ext.len()];
    format!(
        "{}{}{}",
        c.dim(&format!("{dir}/")),
        c.bold(base),
        c.dim(&ext)
    )
}

/// The Failed Suites / Failed Tests sections (stderr), as vitest's printErrorsSummary: each
/// section counts its tasks; identical errors (vitest: the same stack and diff) are printed once
/// under every task's ` FAIL  <name>` line; the dividers number the printed errors out of all the
/// tasks' errors (vitest's counter does not account for merged ones); `verbose` adds each error's
/// properties (vitest's `printProperties: verbose`, turbo-test#97).
fn errors_summary(res: &[FileResult], run: &Run, verbose: bool) -> String {
    let c = &run.colors;
    let (suites, tests) = collect_failed_sections(res, run);
    let total: usize = suites.iter().chain(&tests).map(|t| t.1.len()).sum();
    let mut out = String::new();
    let mut n = 0;
    for (title, tasks) in [("Failed Suites", &suites), ("Failed Tests", &tests)] {
        if tasks.is_empty() {
            continue;
        }
        // errorBanner: divider(c.bold(c.bgRed(` ${title} `)), null, null, c.red)
        let banner = c.bold(&c.bg_red(&format!(" {title} {} ", tasks.len())));
        out.push_str(&format!("\n{}\n\n", divider(&banner, None, &|t| c.red(t))));
        let queue = merge_section_errors(tasks);
        for (error, names, notes, file) in queue {
            n += 1;
            for name in names {
                out.push_str(&format!("{} {name}\n", c.bg_red(&c.bold(" FAIL "))));
            }
            out.push_str(&print_error(error, run.root, true, verbose, "", c));
            if !notes.is_empty() {
                out.push_str(&annotation_lines(c, notes, file, 1));
                out.push('\n');
            }
            let bar = divider(&format!("[{n}/{total}]"), Some(1), &|t| t.to_string());
            out.push_str(&format!("{}\n\n", c.red(&c.dim(&bar))));
        }
    }
    out
}

/// One entry per failed task: `(full name, its errors, its annotations, the file's display name)`.
/// The two sections of `errors_summary` — Failed Suites / Failed Tests.
type FailedTask<'a> = (String, Vec<ErrorInfo>, &'a [Annotation], String);

/// Walk every `FileResult` once and split its failures into the Failed Suites / Failed Tests
/// buckets in the order `errors_summary` prints them. A file whose top-level setup failed goes in
/// suites under its own `[ name ]`; a `SuiteError` goes in suites; a failed test goes in tests.
fn collect_failed_sections<'a>(
    res: &'a [FileResult],
    run: &Run,
) -> (Vec<FailedTask<'a>>, Vec<FailedTask<'a>>) {
    let c = &run.colors;
    let sep = c.dim(" > ");
    let full = |name: &str, path: &str| {
        format!(
            "{name}{sep}{}",
            path.split(" > ").collect::<Vec<_>>().join(&sep)
        )
    };
    let mut suites: Vec<FailedTask<'a>> = Vec::new();
    let mut tests: Vec<FailedTask<'a>> = Vec::new();
    for r in res {
        let name = run.names[r.idx].to_string_lossy().to_string();
        if r.failed_file && r.tests.is_empty() && r.suite_errors.is_empty() {
            let error = r.load_error_info.clone().unwrap_or_else(|| ErrorInfo {
                name: "Error".into(),
                message: crate::cli::file_message(&r.error_msg),
                ..Default::default()
            });
            suites.push((
                format!("{name}{}", c.dim(&format!(" [ {name} ]"))),
                vec![error],
                &[],
                name.clone(),
            ));
        }
        for s in &r.suite_errors {
            suites.push((full(&name, &s.name), s.errors.clone(), &[], name.clone()));
        }
        for t in r.tests.iter().filter(|t| t.status == "failed") {
            let task = full(&name, &test_path(t));
            tests.push((task, errors(t), &t.annotations, name.clone()));
        }
    }
    (suites, tests)
}

/// One merged entry in a section's print queue: `(the error, the names of the tasks that share
/// it, its annotations, the file the first such task belongs to)`.
type MergedError<'q> = (&'q ErrorInfo, Vec<&'q str>, &'q [Annotation], &'q str);

/// Merge identical errors across a section's tasks: an error merges with one already in the queue
/// when both the error and the annotations match. The second task's name is then appended under
/// the first task's `FAIL` banner rather than the error being printed again.
fn merge_section_errors<'q>(tasks: &'q [FailedTask<'_>]) -> Vec<MergedError<'q>> {
    let mut queue: Vec<MergedError<'q>> = Vec::new();
    for (name, errs, notes, file) in tasks.iter() {
        for e in errs {
            match queue.iter_mut().find(|q| q.0 == e && q.2 == *notes) {
                Some(q) => q.1.push(name),
                None => queue.push((e, vec![name], notes, file)),
            }
        }
    }
    queue
}

/// A string's length in UTF-16 code units (JavaScript's `length`).
fn js_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// vitest's `divider` in a 30-column non-terminal: the text (measured without its colours)
/// centred in `⎯`, or with `right` `⎯` after it; `color` paints the `⎯` runs.
fn divider(text: &str, right: Option<usize>, color: &dyn Fn(&str) -> String) -> String {
    let cols = 30usize;
    let len = js_len(&strip(text));
    let (left, right) = match right {
        Some(r) => (cols.saturating_sub(len + r), r),
        None => {
            let left = cols.saturating_sub(len) / 2;
            (left, cols.saturating_sub(len + left))
        }
    };
    format!(
        "{}{text}{}",
        color(&"⎯".repeat(left)),
        color(&"⎯".repeat(right))
    )
}

/// vitest's `printErrorInner` into a string: `Name: message` (red, the name bold; a value
/// without a message as its serialized form, turbo-test#97), the diff, the frames (the nearest,
/// the first outside node_modules, cyan, the others gray) with the highlighted code frame after
/// the nearest one (`code_frame`), a blank line, the error's properties as `Serialized Error` under a
/// divider (`properties`: vitest's `printProperties`, the verbose reporter's and junit's), the
/// cause (its properties always: vitest prints the cause with the default).
pub fn print_error(
    e: &ErrorInfo,
    root: &Path,
    code_frame: bool,
    properties: bool,
    prefix: &str,
    c: &Colors,
) -> String {
    let name = format!("{prefix}{}", e.name);
    let mut out = if let Some(shown) = &e.inspected {
        format!("{shown}\n")
    } else if js_len(&e.message) > 5000 {
        format!("{}: {}\n", c.red(&c.bold(&name)), e.message)
    } else {
        c.red(&format!("{}: {}", c.bold(&name), e.message)) + "\n"
    };
    // vite's details (`__vitest_rollup_error__`): `  Plugin: …`, `  File: <id>[:line:col]` and
    // its code frame indented two spaces (turbo-test#113)
    if let Some(r) = &e.rollup {
        out.push_str(&rollup_details(r, c));
    }
    if !e.diff.is_empty() {
        out.push_str(&format!("\n{}\n\n", e.diff));
    }
    out.push_str(&render_frames(e, root, code_frame, c));
    if let (true, Some(props)) = (properties, &e.properties) {
        // printStack: c.red(c.dim(divider())), then c.red(c.bold('Serialized Error:')), c.gray(…)
        out.push_str(&format!(
            "{}\n{} {}\n",
            c.red(&c.dim(&divider("", None, &|t| t.to_string()))),
            c.red(&c.bold("Serialized Error:")),
            c.gray(props)
        ));
    }
    if let Some(cause) = &e.cause {
        out.push_str(&print_error(cause, root, false, true, "Caused by: ", c));
    }
    out
}

/// Render the `❯ method file:line:col` frame lines for `e.frames`, the highlighted code frame
/// for the nearest frame (when `code_frame` is on), and the trailing blank line. The nearest
/// frame is the first `FileResult` in a file vite transformed — not an external `node_modules`
/// one (turbo-test#173); it is cyan, the others are gray.
fn render_frames(e: &ErrorInfo, root: &Path, code_frame: bool, c: &Colors) -> String {
    let frames: Vec<&Frame> = e.frames.iter().collect();
    let nearest = frames.iter().position(|f| {
        f.file.is_file() && !f.file.components().any(|c| c.as_os_str() == "node_modules")
    });
    let mut out = String::new();
    for (i, f) in frames.iter().enumerate() {
        let path = f.file.strip_prefix(root).unwrap_or(&f.file);
        let at = format!(
            "{}:{}",
            path.display(),
            c.dim(&format!("{}:{}", f.line, f.col))
        );
        let pos = if f.method.is_empty() {
            at
        } else {
            format!("{} {at}", f.method)
        };
        let line = format!(" {} {pos}", c.dim("❯"));
        out.push_str(&if Some(i) == nearest {
            c.cyan(&line)
        } else {
            c.gray(&line)
        });
        out.push('\n');
        if code_frame && Some(i) == nearest {
            out.push_str(&nearest_code_frame(f, c));
        }
    }
    if !frames.is_empty() {
        out.push('\n');
    }
    out
}

/// vite's rollup error details: `  Plugin: …`, `  File: <id>[:line:col]` and its code frame
/// indented two spaces, each on a line of its own ("" when it has none).
fn rollup_details(r: &crate::failure::RollupDetails, c: &Colors) -> String {
    let mut lines = Vec::new();
    if !r.plugin.is_empty() {
        lines.push(format!("  Plugin: {}", c.magenta(&r.plugin)));
    }
    if !r.id.is_empty() {
        let loc = if r.loc.is_empty() {
            String::new()
        } else {
            format!(":{}", r.loc)
        };
        lines.push(format!("  File: {}{loc}", c.cyan(&r.id)));
    }
    if !r.frame.is_empty() {
        let frame: Vec<String> = r
            .frame
            .split('\n')
            .map(|l| format!("  {}", l.strip_suffix('\r').unwrap_or(l)))
            .collect();
        lines.push(c.yellow(&frame.join("\n")));
    }
    if lines.is_empty() {
        return String::new();
    }
    lines.join("\n") + "\n"
}

/// The highlighted code frame of the nearest frame `f` and a line break ("" when its file
/// cannot be read).
fn nearest_code_frame(f: &Frame, c: &Colors) -> String {
    std::fs::read_to_string(&f.file).map_or(String::new(), |source| {
        let shown = if js_len(&source) > 100_000 {
            source
        } else {
            crate::highlight::highlight_code(c, &f.file, &source)
        };
        code_frame_of(&shown, f.line, f.col, c) + "\n"
    })
}

/// vitest's `generateCodeFrame(source, 4, { line, column })` for an 80-column output, over the
/// (highlighted) source: the line `positionToOffset` lands on and two either side, each cut to 71
/// characters with `…` (then without its colours), gray line numbers and a red caret under the
/// column. A source-map comment line is left out; a line over 200 characters drops the frame.
pub fn code_frame_of(source: &str, line: u32, column: u32, c: &Colors) -> String {
    let lines: Vec<&str> = source
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let nl = if source.contains("\r\n") { 2 } else { 1 };
    // positionToOffset
    let start = if line as usize > lines.len() {
        js_len(source)
    } else {
        lines[..(line as usize).saturating_sub(1)]
            .iter()
            .map(|l| js_len(l) + nl)
            .sum::<usize>()
            + column as usize
    };
    let no = |n: Option<usize>| c.gray(&n.map_or("   |".to_string(), |n| format!("{n:>3}|")));
    let mut count = 0;
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        count += js_len(l) + nl;
        if count < start {
            continue;
        }
        let lo = i.saturating_sub(2);
        let hi = (i + 2).min(lines.len() - 1);
        if !emit_frame_window(&lines, &mut out, lo, hi, i, &no, c, start, count, l, nl) {
            return String::new();
        }
        break;
    }
    out.iter()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Emit the (up to) five-line window centered on line `i`. Returns `false` if any line in the
/// window is over 200 chars (dropping the whole frame, matching vitest).
#[allow(clippy::too_many_arguments)]
fn emit_frame_window(
    lines: &[&str],
    out: &mut Vec<String>,
    lo: usize,
    hi: usize,
    i: usize,
    no: &impl Fn(Option<usize>) -> String,
    c: &Colors,
    start: usize,
    count: usize,
    l: &str,
    nl: usize,
) -> bool {
    for (j, text) in lines.iter().enumerate().take(hi + 1).skip(lo) {
        let plain = strip(text);
        if plain.starts_with("//# sourceMappingURL") {
            continue;
        }
        if js_len(&plain) > 200 {
            return false;
        }
        let truncated = truncate_frame_line(text);
        let truncated = truncated.trim_end();
        out.push(if truncated.is_empty() {
            no(Some(j + 1))
        } else {
            format!("{} {truncated}", no(Some(j + 1)))
        });
        if j == i {
            // `start - (count - lineLength) + (nl - 1)`, repeated `pad + 1` times
            let pad = start as i64 - (count - js_len(l)) as i64 + (nl as i64 - 1);
            let spaces = " ".repeat((pad + 1).max(0) as usize);
            out.push(format!("{}{spaces}{}", no(None), c.red("^")));
        }
    }
    true
}

/// Replace tabs with a space and cut over-wide lines to 70 chars + `…`, preserving ANSI colours.
fn truncate_frame_line(text: &str) -> String {
    let tabbed = text.replace('\t', " ");
    if js_len(&strip(&tabbed)) <= 71 {
        return tabbed;
    }
    let mut units = 0;
    let cut: String = strip(&tabbed)
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= 70
        })
        .collect();
    format!("{cut}…")
}

#[cfg(test)]
#[path = "human_tests.rs"]
mod tests;
