//! Native code coverage via V8's Inspector `Profiler` precise-coverage domain.
//!
//! Everything here is gated behind `enabled()` (the `--coverage` flag) — when off, the runner
//! never constructs a Collector, names a script, or emits a source map, so the default fast path
//! is byte-identical. V8 precise coverage is near-zero runtime overhead (no Istanbul-style
//! instrumentation): the engine records per-function call counts and block ranges internally, we
//! just read them out at the end of each file and remap the byte ranges back to original `.ts`
//! lines through the esbuild source map we emit (under coverage only).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;

static ENABLED: AtomicBool = AtomicBool::new(false);
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

// Output directory for the lcov report (default: <cwd>/coverage). `--coverage-dir <path>`.
static OUT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
pub fn set_out_dir(dir: &str) {
    *OUT_DIR.lock().unwrap() = Some(PathBuf::from(dir));
}
fn out_dir() -> PathBuf {
    if let Some(d) = OUT_DIR.lock().unwrap().clone() {
        return d;
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("coverage")
}

// ---- report config: thresholds, reporters, include/exclude (M6 coverage gating) -------------
//
// `statements` is derived (V8 has no native statement counter): the original source is parsed
// with oxc, and each executable statement's position is correlated with V8's covered byte ranges
// — the same machinery as branches. This mirrors how c8 reports statements, so vitest/c8 users
// get the metric they expect. (lcov has no statement field, so it appears in json-summary / text
// / html only.)

// Per-metric minimum percentages (0..=100). None = not gated on that metric.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Thresholds {
    pub lines: Option<f64>,
    pub functions: Option<f64>,
    pub branches: Option<f64>,
    pub statements: Option<f64>,
}
static THRESHOLDS: Mutex<Option<Thresholds>> = Mutex::new(None);
static PER_FILE: AtomicBool = AtomicBool::new(false);

/// Parse `lines=90,functions=80,branches=80,statements=90` (any subset, any order). Unknown
/// keys ignored.
pub fn set_thresholds(spec: &str) {
    let t = THRESHOLDS.lock().unwrap().unwrap_or_default();
    let t = parse_thresholds(t, spec);
    *THRESHOLDS.lock().unwrap() = Some(t);
}
/// Apply a threshold spec on top of `t` (later keys win; malformed parts are skipped).
fn parse_thresholds(mut t: Thresholds, spec: &str) -> Thresholds {
    for part in spec.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let Ok(n) = v.trim().parse::<f64>() else {
            continue;
        };
        match k.trim() {
            "lines" => t.lines = Some(n),
            "functions" | "funcs" => t.functions = Some(n),
            "branches" => t.branches = Some(n),
            "statements" | "stmts" => t.statements = Some(n),
            _ => {}
        }
    }
    t
}
pub fn set_per_file(on: bool) {
    PER_FILE.store(on, Ordering::Relaxed);
}

// Reporters to emit. Default lcov + text (the historical behavior) when none specified.
static REPORTERS: Mutex<Option<Vec<String>>> = Mutex::new(None);
pub fn set_reporters(spec: &str) {
    let list: Vec<String> = spec
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if !list.is_empty() {
        *REPORTERS.lock().unwrap() = Some(list);
    }
}
fn reporters() -> Vec<String> {
    // Default set: lcov + json-summary + text. HTML is opt-in only (`--coverage-reporter html`)
    // since it writes a browsable site most CI runs don't need.
    REPORTERS.lock().unwrap().clone().unwrap_or_else(|| {
        vec![
            "lcov".to_string(),
            "json-summary".to_string(),
            "text".to_string(),
        ]
    })
}

// include/exclude globs (cwd-relative, vitest `coverage.include`/`coverage.exclude`). Empty
// include = report everything (current behavior); exclude is always applied.
static INCLUDE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EXCLUDE: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Split a comma-joined glob list on TOP-LEVEL commas only — commas inside `{a,b}` brace
/// alternation are part of a single glob, not separators. Without this, the canonical vitest
/// include `src/**/*.{ts,tsx}` (forwarded comma-joined) was split into `src/**/*.{ts` + `tsx}`,
/// two malformed globs that match nothing → silent zero coverage.
fn split_globs(spec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in spec.chars() {
        match ch {
            '{' => {
                depth += 1;
                cur.push(ch);
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                }
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let t = cur.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}
pub fn add_include(spec: &str) {
    let mut g = INCLUDE.lock().unwrap();
    g.extend(split_globs(spec));
}
pub fn add_exclude(spec: &str) {
    let mut g = EXCLUDE.lock().unwrap();
    g.extend(split_globs(spec));
}

// Files carrying a per-file ignore directive (P5) — excluded from the report + the gate.
static IGNORED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
fn mark_ignored(file: &Path) {
    IGNORED
        .lock()
        .unwrap()
        .get_or_insert_with(HashSet::new)
        .insert(file.to_path_buf());
}
fn is_ignored(file: &Path) -> bool {
    IGNORED
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.contains(file))
        .unwrap_or(false)
}

/// Honor a magic comment in a source file's leading lines that exempts it from coverage
/// reporting + gating. Recognized (anywhere in the first ~10 lines):
///   `turbo-test-coverage-ignore-file`, `coverage-ignore-file`,
///   or the consumer's legacy `disable-test-coverage-check`.
fn scan_ignore_directive(file: &Path, src: &str) {
    const MARKERS: [&str; 3] = [
        "turbo-test-coverage-ignore-file",
        "coverage-ignore-file",
        "disable-test-coverage-check",
    ];
    for line in src.lines().take(10) {
        if MARKERS.iter().any(|m| line.contains(m)) {
            mark_ignored(file);
            return;
        }
    }
}

// ---- glob matching (vitest-style: **, *, ?, {a,b}) against a cwd-relative POSIX path ---------

/// Expand single-level `{a,b,c}` alternations into concrete patterns (no nesting), then test each
/// expansion. Sufficient for the globs vitest configs use (`src/**/*.{ts,tsx}`).
fn glob_match(pat: &str, path: &str) -> bool {
    expand_braces(pat)
        .iter()
        .any(|p| simple_match(p.as_bytes(), path.as_bytes()))
}

/// Public reuse for the CLI launcher's vitest include/exclude discovery: vitest matches the same
/// `**`/`*`/`?`/`{a,b}` glob shape against a project-root-relative POSIX path. Same matcher → the
/// launcher's discovery and coverage's file filtering agree on what a glob means.
pub fn glob_matches(pat: &str, path: &str) -> bool {
    glob_match(pat, path)
}

fn expand_braces(pat: &str) -> Vec<String> {
    let Some(open) = pat.find('{') else {
        return vec![pat.to_string()];
    };
    let Some(rel_close) = pat[open..].find('}') else {
        return vec![pat.to_string()];
    };
    let close = open + rel_close;
    let (pre, inner, post) = (&pat[..open], &pat[open + 1..close], &pat[close + 1..]);
    let mut out = Vec::new();
    for alt in inner.split(',') {
        // recurse to expand any further braces in the tail
        for tail in expand_braces(&format!("{alt}{post}")) {
            out.push(format!("{pre}{tail}"));
        }
    }
    out
}

/// Match a brace-free glob (`**` spans path separators, `*`/`?` do not) against a path.
fn simple_match(pat: &[u8], path: &[u8]) -> bool {
    let (mut pi, mut si) = (0usize, 0usize);
    // backtrack state for the most recent `*` (single-segment star)
    let (mut star, mut star_si): (Option<usize>, usize) = (None, 0);
    while si < path.len() {
        if pi < pat.len() {
            match pat[pi] {
                b'*' => {
                    if pi + 1 < pat.len() && pat[pi + 1] == b'*' {
                        // `**` — matches across separators. Greedy with backtracking via outer loop:
                        // try to consume the rest after `**` (skip an optional trailing '/').
                        let mut rest = pi + 2;
                        if rest < pat.len() && pat[rest] == b'/' {
                            rest += 1;
                        }
                        if rest >= pat.len() {
                            return true; // trailing `**` matches anything
                        }
                        // attempt to match the remainder at each position to end of path
                        for k in si..=path.len() {
                            if simple_match(&pat[rest..], &path[k..]) {
                                return true;
                            }
                        }
                        return false;
                    }
                    // single `*` — matches zero+ non-separator chars
                    star = Some(pi);
                    star_si = si;
                    pi += 1;
                    continue;
                }
                b'?' => {
                    if path[si] != b'/' {
                        pi += 1;
                        si += 1;
                        continue;
                    }
                }
                c => {
                    if c == path[si] {
                        pi += 1;
                        si += 1;
                        continue;
                    }
                }
            }
        }
        // mismatch — backtrack into the last single `*` if any (but never across '/')
        if let Some(sp) = star {
            if path[star_si] != b'/' {
                pi = sp + 1;
                star_si += 1;
                si = star_si;
                continue;
            }
        }
        return false;
    }
    // consume trailing `*` / `**` in the pattern
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

/// A path as a vite/vitest module id: `/` separators. Only Windows rewrites `\\` (on POSIX a
/// backslash is a legal file-name character).
pub fn slash_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        // vite's normalizePath: `C:/x/y` (a verbatim `\\?\` prefix is dropped)
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

/// The file-system form of a `/`-separated id path (Windows verbatim `\\?\` paths reject `/`).
pub fn native_path(s: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(s.replace('/', "\\"))
    } else {
        PathBuf::from(s)
    }
}

/// Whether a file passes the configured include/exclude globs. Matched against the file's
/// cwd-relative POSIX path (how vitest interprets `coverage.include`).
fn passes_globs(file: &Path) -> bool {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let rel = file.strip_prefix(&cwd).unwrap_or(file);
    let rel = rel.to_string_lossy().replace('\\', "/");
    let inc = INCLUDE.lock().unwrap();
    let exc = EXCLUDE.lock().unwrap();
    glob_filter(&rel, &inc, &exc)
}

/// Include/exclude decision for a cwd-relative POSIX path: empty include = everything.
fn glob_filter(rel: &str, inc: &[String], exc: &[String]) -> bool {
    if !inc.is_empty() && !inc.iter().any(|g| glob_match(g, rel)) {
        return false;
    }
    if exc.iter().any(|g| glob_match(g, rel)) {
        return false;
    }
    true
}

/// Per-file coverage records: file -> (key -> value).
type PerFile<K, V> = Mutex<Option<HashMap<PathBuf, HashMap<K, V>>>>;
/// A branch: (block, branch).
type BranchKey = (u32, u32);
/// A branch's record: (line, reached, max taken).
type BranchHit = (u32, bool, u64);

// file -> (origLine -> max hit count). Presence of a line = it's executable; value > 0 = covered.
static COV: PerFile<u32, u64> = Mutex::new(None);
// file -> ((origLine, fnName) -> max call count). Function coverage.
static FN_COV: PerFile<(u32, String), u64> = Mutex::new(None);
// file -> ((block, branch) -> (line, reached, max taken)). Branch coverage.
static BR_COV: PerFile<BranchKey, BranchHit> = Mutex::new(None);
// file -> (statement index -> (line, max hit count)). Statement coverage.
static ST_COV: PerFile<u32, (u32, u64)> = Mutex::new(None);

/// Record one statement's hit count. Merges across the many test files exercising the same source
/// (a statement is covered if any run hit it — max count).
pub fn record_stmt(file: &Path, idx: u32, line: u32, count: u64) {
    let mut g = ST_COV.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let f = m.entry(file.to_path_buf()).or_default();
    let e = f.entry(idx).or_insert((line, 0));
    e.0 = line;
    if count > e.1 {
        e.1 = count;
    }
}

/// Record one branch arm's outcome. Merges across the many test files that exercise the same
/// source: a branch is reached/taken if any run reached/took it (max counts).
pub fn record_branch(file: &Path, block: u32, branch: u32, line: u32, reached: bool, taken: u64) {
    let mut g = BR_COV.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let f = m.entry(file.to_path_buf()).or_default();
    let e = f.entry((block, branch)).or_insert((line, false, 0));
    e.0 = line;
    e.1 |= reached;
    if taken > e.2 {
        e.2 = taken;
    }
}

pub fn record(file: &Path, line: u32, count: u64) {
    let mut g = COV.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let f = m.entry(file.to_path_buf()).or_default();
    let e = f.entry(line).or_insert(0);
    if count > *e {
        *e = count;
    }
}

pub fn record_fn(file: &Path, line: u32, name: &str, count: u64) {
    let mut g = FN_COV.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let f = m.entry(file.to_path_buf()).or_default();
    let e = f.entry((line, name.to_string())).or_insert(0);
    if count > *e {
        *e = count;
    }
}

// Per-source-file static map data (line-start table + gen→src line map). Built ONCE per file and
// reused across every test file that covers it — the source map is the same each time (the
// transform is cache-backed), so re-decoding it per occurrence is pure waste.
// a branch decision in original (line, col) coordinates.
struct BranchCC {
    decision: (u32, u32),
    arms: Vec<(u32, u32)>,
    implicit_else: bool,
}

struct ScriptMeta {
    line_start: Vec<usize>,    // UTF-16 line starts of the WRAPPED compiled source
    genmap: HashMap<u32, u32>, // gen line -> src line (first segment) — line coverage
    segments: Vec<Vec<(u32, u32, u32)>>, // per gen line: (gen_col, src_line, src_col) — gen→orig
    branches: Vec<BranchCC>,   // decision points in original (line, col)
    statements: Vec<(u32, u32)>, // executable statement starts in original (line, col)
    /// gen lines `[0, header_end)` are esbuild's interop prologue (see `interop_header_end`)
    header_end: usize,
    /// wrapped offsets of the first and last mapped positions of the code (see `code_span`)
    code_span: Option<(usize, usize)>,
    /// per gen line, per segment: blank lead-in columns of its run (see `segment_leads`)
    leads: Vec<Vec<u32>>,
    /// per gen line: its first segment covers blanks only (see `blank_first_segments`)
    blank_first: Vec<bool>,
}

impl ScriptMeta {
    /// The 0-based WRAPPED line holding `off` when that line is generated source code — not the
    /// wrapper line 0 and not esbuild's interop prologue (see `interop_header_end`).
    fn code_line(&self, off: usize) -> Option<usize> {
        let li = self
            .line_start
            .partition_point(|&s| s <= off)
            .saturating_sub(1);
        (li > self.header_end).then_some(li) // wrapped line li ↔ esbuild gen line li-1
    }
}

/// esbuild's ESM→CJS interop prologue: the runtime helpers, `var x_exports = {};`, the
/// `__export(x_exports, { name: () => name, … })` getter table and
/// `module.exports = __toCommonJS(x_exports);`. esbuild maps every mapped line of it to source
/// (0, 0) only, so it would otherwise read as code on source line 1 — and every export getter as
/// a function there. Returns the number of leading generated lines it spans: up to the
/// `module.exports = __toCommonJS(` line, provided nothing before it maps anywhere but (0, 0).
/// 0 when there is none (a CommonJS source, or an ESM one without exports).
fn interop_header_end(gen_lines: &[&str], segs: &Segments) -> usize {
    for (i, (row, line)) in segs.iter().zip(gen_lines).enumerate() {
        if row.iter().any(|s| (s.1, s.2) != (0, 0)) {
            return 0;
        }
        if line.starts_with("module.exports = __toCommonJS(") {
            return i + 1;
        }
    }
    0
}

/// Wrapped offsets of the first and last mapped generated positions after the prologue (None
/// when nothing is mapped). V8 ranges that open before / close after them are the module-wide
/// ones — see `orig_ranges`.
fn code_span(line_start: &[usize], segs: &Segments, header_end: usize) -> Option<(usize, usize)> {
    let mut mapped = segs
        .iter()
        .enumerate()
        .skip(header_end)
        .filter_map(|(g, row)| {
            let ls = *line_start.get(g + 1)?; // esbuild gen line g ↔ wrapped line g+1
            Some((ls + row.first()?.0 as usize, ls + row.last()?.0 as usize))
        });
    let first = mapped.next()?;
    let last = mapped.next_back().unwrap_or(first);
    Some((first.0, last.1))
}
/// Per segment (same shape as `segs`): how many leading columns of its generated run are blanks
/// that are not source text. esbuild often starts a segment on the indentation or on the space
/// before the token it maps — `  if (c) throw …` maps gen col 0 to the `if` at source col 2 and
/// gen col 8 (the space) to the `throw` at col 9 — so a plain 1:1 offset delta put every later
/// column of the run that far right of its token (V8's range for the `throw` started a column
/// past it). Only blanks in front of a non-blank source position count: there they cannot be
/// part of the source run.
fn segment_leads(gen_lines: &[&[u16]], orig_lines: &[&[u16]], segs: &Segments) -> Vec<Vec<u32>> {
    let blank = |c: &u16| *c == 0x20 || *c == 0x09;
    let mut out = Vec::with_capacity(segs.len());
    for (g, row) in segs.iter().enumerate() {
        let text = gen_lines.get(g).copied().unwrap_or_default();
        let mut leads = Vec::with_capacity(row.len());
        for (i, &(col, line, src_col)) in row.iter().enumerate() {
            let end = row.get(i + 1).map_or(text.len(), |n| n.0 as usize);
            let run = text.get(col as usize..end).unwrap_or_default();
            let w = run.iter().take_while(|c| blank(c)).count() as u32;
            let at = orig_lines
                .get(line as usize)
                .and_then(|t| t.get(src_col as usize));
            leads.push(if at.is_some_and(|c| !blank(c)) { w } else { 0 });
        }
        out.push(leads);
    }
    out
}

/// Per gen line: whether its first segment's generated run is blank only. esbuild opens such a
/// line (`    case "b":`, `    default:`, `  }`) with a segment for its indentation that points
/// at where the PREVIOUS source token ended (`    default:` → the `2` of `return 2;` on the line
/// before), and its code with a later segment. Line coverage attributes a generated line to its
/// first segment's source line, sampled at the line start, so such a line used to credit the
/// previous source line with a count taken outside every case range.
fn blank_first_segments(gen_lines: &[&[u16]], segs: &Segments) -> Vec<bool> {
    let blank = |c: &u16| *c == 0x20 || *c == 0x09;
    let mut out = Vec::with_capacity(segs.len());
    for (g, row) in segs.iter().enumerate() {
        let text = gen_lines.get(g).copied().unwrap_or_default();
        let run = match row.first() {
            Some(first) => {
                let end = row.get(1).map_or(text.len(), |n| n.0 as usize);
                text.get(first.0 as usize..end).unwrap_or_default()
            }
            None => &[],
        };
        out.push(!run.is_empty() && run.iter().all(blank));
    }
    out
}

static META: Mutex<Option<HashMap<PathBuf, Option<Arc<ScriptMeta>>>>> = Mutex::new(None);

pub fn has_meta(file: &Path) -> bool {
    META.lock()
        .unwrap()
        .as_ref()
        .map(|m| m.contains_key(file))
        .unwrap_or(false)
}

/// Build + cache the static map data for a source file from its wrapped compiled source + the
/// original source (for branch AST). Stores `None` if the source map can't be parsed.
pub fn register_meta(file: &Path, wrapped: &str, orig_source: &str) {
    scan_ignore_directive(file, orig_source);
    let meta = decode_inline_map(wrapped).map(|(genmap, segments)| {
        let u16s: Vec<u16> = wrapped.encode_utf16().collect();
        let mut line_start = vec![0usize];
        for (i, &c) in u16s.iter().enumerate() {
            if c == 0x0A {
                line_start.push(i + 1);
            }
        }
        // parse the original source ONCE → branch decision points + statement starts, in (line, col).
        let (raw_branches, raw_stmts) = crate::coverage_branch::extract_all(file, orig_source);
        let branches = raw_branches
            .into_iter()
            .map(|b| BranchCC {
                decision: byte_to_linecol(orig_source, b.decision),
                arms: b
                    .arms
                    .iter()
                    .map(|&o| byte_to_linecol(orig_source, o))
                    .collect(),
                implicit_else: b.implicit_else,
            })
            .collect();
        let statements = raw_stmts
            .into_iter()
            .map(|o| byte_to_linecol(orig_source, o))
            .collect();
        let gen_lines: Vec<&str> = wrapped.split('\n').skip(1).collect();
        let header_end = interop_header_end(&gen_lines, &segments);
        let code_span = code_span(&line_start, &segments, header_end);
        let gen16: Vec<&[u16]> = u16s.split(|&c| c == 0x0A).skip(1).collect();
        let orig_u16: Vec<u16> = orig_source.encode_utf16().collect();
        let orig16: Vec<&[u16]> = orig_u16.split(|&c| c == 0x0A).collect();
        let leads = segment_leads(&gen16, &orig16, &segments);
        let blank_first = blank_first_segments(&gen16, &segments);
        Arc::new(ScriptMeta {
            line_start,
            genmap,
            segments,
            branches,
            statements,
            header_end,
            code_span,
            leads,
            blank_first,
        })
    });
    let mut g = META.lock().unwrap();
    g.get_or_insert_with(HashMap::new)
        .insert(file.to_path_buf(), meta);
}

fn meta_for(file: &Path) -> Option<Arc<ScriptMeta>> {
    META.lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(file).cloned())
        .flatten()
}

/// Original-source byte offset → (line, UTF-16 column), both 0-based.
fn byte_to_linecol(src: &str, off: u32) -> (u32, u32) {
    let (mut line, mut col, mut b) = (0u32, 0u32, 0u32);
    for ch in src.chars() {
        if b >= off {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }
        b += ch.len_utf8() as u32;
    }
    (line, col)
}

// ---- inspector session (one per worker isolate, while coverage is on) -----------------------

struct CovChannel {
    resp: Rc<RefCell<Option<String>>>,
}
impl v8::inspector::ChannelImpl for CovChannel {
    fn send_response(&self, _call_id: i32, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        if let Some(m) = message.as_ref() {
            *self.resp.borrow_mut() = Some(format!("{}", m.string()));
        }
    }
    fn send_notification(&self, _message: v8::UniquePtr<v8::inspector::StringBuffer>) {}
    fn flush_protocol_notifications(&self) {}
}

struct CovClient;
impl v8::inspector::V8InspectorClientImpl for CovClient {}

pub struct Collector {
    inspector: v8::inspector::V8Inspector,
    session: Option<v8::inspector::V8InspectorSession>,
    resp: Rc<RefCell<Option<String>>>,
}

impl Collector {
    pub fn new(isolate: &mut v8::Isolate) -> Self {
        let client = v8::inspector::V8InspectorClient::new(Box::new(CovClient));
        let inspector = v8::inspector::V8Inspector::create(isolate, client);
        Collector {
            inspector,
            session: None,
            resp: Rc::new(RefCell::new(None)),
        }
    }

    /// Register the context with the inspector and begin precise coverage. Call once, right
    /// after the context exists and before any module loads.
    pub fn start(&mut self, context: v8::Local<v8::Context>) {
        self.inspector.context_created(
            context,
            1,
            v8::inspector::StringView::from(&b"turbo-test"[..]),
            v8::inspector::StringView::from(&b"{}"[..]),
        );
        let channel = v8::inspector::Channel::new(Box::new(CovChannel {
            resp: self.resp.clone(),
        }));
        let session = self.inspector.connect(
            1,
            channel,
            v8::inspector::StringView::empty(),
            v8::inspector::V8InspectorClientTrustLevel::FullyTrusted,
        );
        dispatch(&session, br#"{"id":1,"method":"Profiler.enable"}"#);
        // detailed=true → block ranges (within-function line precision). Needed for honest line
        // coverage; function coverage falls out of each function's outer range regardless.
        dispatch(&session, br#"{"id":2,"method":"Profiler.startPreciseCoverage","params":{"callCount":true,"detailed":true}}"#);
        self.session = Some(session);
    }

    /// Take the accumulated coverage JSON (Profiler.takePreciseCoverage result). The inspector
    /// answers synchronously via the channel during dispatch, so the response is ready on return.
    pub fn take(&mut self) -> Option<String> {
        let session = self.session.as_ref()?;
        self.resp.borrow_mut().take();
        dispatch(
            session,
            br#"{"id":3,"method":"Profiler.takePreciseCoverage"}"#,
        );
        self.resp.borrow_mut().take()
    }

    /// Disconnect the session and unregister the context — MUST run while the context + isolate
    /// are still alive, otherwise the inspector's teardown dereferences freed state (segfault).
    pub fn stop(&mut self, context: v8::Local<v8::Context>) {
        self.session = None; // disconnect before the context goes away
        self.inspector.context_destroyed(context);
    }
}

fn dispatch(session: &v8::inspector::V8InspectorSession, msg: &[u8]) {
    session.dispatch_protocol_message(v8::inspector::StringView::from(msg));
}

// ---- source-map remap (hand-rolled VLQ — avoids a network crate dependency) -----------------

/// Extract the esbuild inline source map's `mappings` and build a `genLine -> srcLine` table
/// (the original line of each generated line's first mapped segment). 0-based line numbers.
type GenMap = HashMap<u32, u32>;
type Segments = Vec<Vec<(u32, u32, u32)>>;

/// Extract the esbuild inline source map and decode it into (gen line → src line) for line
/// coverage and per-gen-line segments (gen_col, src_line, src_col) for gen→orig position mapping.
fn decode_inline_map(transformed: &str) -> Option<(GenMap, Segments)> {
    let marker = "//# sourceMappingURL=data:application/json;base64,";
    let pos = transformed.rfind(marker)?;
    let b64 = transformed[pos + marker.len()..].lines().next()?.trim();
    let json = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&json).ok()?;
    let mappings = v.get("mappings")?.as_str()?;
    Some(parse_mappings_full(mappings))
}

/// The per-generated-line segments of `code`'s inline source map (test locations, location.rs).
pub(crate) fn inline_segments(code: &str) -> Option<Segments> {
    decode_inline_map(code).map(|(_, segs)| segs)
}

/// The per-generated-line segments of a source map's `mappings` string.
pub(crate) fn mappings_segments(mappings: &str) -> Segments {
    parse_mappings_full(mappings).1
}

fn parse_mappings_full(mappings: &str) -> (GenMap, Segments) {
    let mut genmap = HashMap::new();
    let mut segs: Segments = Vec::new();
    // src_line/col/idx are cumulative across the WHOLE mappings string; gen_col resets per line.
    let (mut src_idx, mut src_line, mut src_col) = (0i64, 0i64, 0i64);
    for (gen_line, line) in mappings.split(';').enumerate() {
        let mut gen_col = 0i64;
        let mut row: Vec<(u32, u32, u32)> = Vec::new();
        let mut first: Option<i64> = None;
        for seg in line.split(',') {
            if seg.is_empty() {
                continue;
            }
            let vals = vlq_decode(seg);
            if vals.is_empty() {
                continue;
            }
            gen_col += vals[0];
            if vals.len() >= 4 {
                src_idx += vals[1];
                src_line += vals[2];
                src_col += vals[3];
                let _ = src_idx;
                if src_line >= 0 && src_col >= 0 && gen_col >= 0 {
                    row.push((gen_col as u32, src_line as u32, src_col as u32));
                }
                if first.is_none() && src_line >= 0 {
                    first = Some(src_line);
                }
            }
        }
        if let Some(sl) = first {
            genmap.insert(gen_line as u32, sl as u32);
        }
        segs.push(row);
    }
    (genmap, segs)
}

fn vlq_decode(seg: &str) -> Vec<i64> {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let val = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as i64);
    let mut out = Vec::new();
    let (mut shift, mut acc) = (0i64, 0i64);
    for &c in seg.as_bytes() {
        let Some(d) = val(c) else { continue };
        let cont = d & 32;
        acc += (d & 31) << shift;
        if cont != 0 {
            shift += 5;
        } else {
            let neg = acc & 1;
            let mut value = acc >> 1;
            if neg != 0 {
                value = -value;
            }
            out.push(value);
            shift = 0;
            acc = 0;
        }
    }
    out
}

/// Given the WRAPPED module source V8 compiled (the CJS function wrapper around the transformed
/// code) and V8's covered byte ranges, attribute hit counts to original source lines.
///
/// `wrapped` line 0 is the `(function (...) {` wrapper, so generated-output line N corresponds to
/// wrapped line N+1; we subtract that one-line wrapper offset before the source-map lookup.
pub fn map_script(file: &Path, ranges: &[(usize, usize, i64)], funcs: &[(String, usize, i64)]) {
    let Some(meta) = meta_for(file) else { return };
    let line_start = &meta.line_start;
    let genmap = &meta.genmap;

    // wrapped line `li` (>= 1) → original source line (1-based), via esbuild gen line li-1
    let src_line_of = |li: usize| genmap.get(&((li - 1) as u32)).map(|&sl| sl + 1);

    // innermost covered range for an offset = the one with the largest start that still contains it.
    let innermost_gen = |off: usize| -> Option<i64> {
        let mut best: Option<(usize, i64)> = None;
        for &(s, e, c) in ranges {
            if s <= off && off < e && best.map(|(bs, _)| s >= bs).unwrap_or(true) {
                best = Some((s, c));
            }
        }
        best.map(|(_, c)| c)
    };

    // line coverage
    let oranges = orig_ranges(&meta, ranges);
    for &off in line_start {
        // the wrapper line and esbuild's interop prologue (mapped to (0, 0), so it would make
        // even a blank or type-only line 1 look executable) are not source lines
        let Some(li) = meta.code_line(off) else {
            continue;
        };
        // A line whose first segment is only its indentation (a `case`/`default`/`}` line) is
        // credited to the source token that segment points back to — the end of the previous
        // line. Take that token's count: the line-start sample lies outside a case range that
        // ends at that token, so it would read the enclosing count instead.
        let count = if meta.blank_first.get(li - 1).copied().unwrap_or(false) {
            let (_, line, col) = meta.segments[li - 1][0];
            innermost(&oranges, (line, col)).map(|r| r.2)
        } else {
            innermost_gen(off)
        };
        let Some(count) = count else { continue };
        if let Some(src_line) = src_line_of(li) {
            record(file, src_line, count.max(0) as u64);
        }
    }

    // function coverage — each function's outer range start offset → its declaration line. A
    // function in esbuild's interop prologue (an export getter `name: () => name`) is not one of
    // the source's functions.
    for (name, start, count) in funcs {
        let Some(li) = meta.code_line(*start) else {
            continue;
        };
        if let Some(src_line) = src_line_of(li) {
            let n = if name.is_empty() {
                "(anonymous)"
            } else {
                name.as_str()
            };
            record_fn(file, src_line, n, (*count).max(0) as u64);
        }
    }
}

/// A WRAPPED generated byte offset → original (src_line, src_col), both 0-based, via the source map
/// segments. Within a segment the offset delta is applied 1:1 (esbuild segments are identity runs)
/// once past the segment's blank lead-in (see `segment_leads`); the lead-in itself maps to the
/// segment's source position.
/// Offsets on the wrapper line or in esbuild's interop prologue have no source position: the
/// prologue's (0, 0) mappings would otherwise put e.g. an unused export getter's 0-count range
/// over source line 1.
fn gen_to_orig(meta: &ScriptMeta, off: usize) -> Option<(u32, u32)> {
    let li = meta.code_line(off)?;
    let gen_col = (off - meta.line_start[li]) as u32;
    let segs = meta.segments.get(li - 1)?; // wrapped line li → esbuild gen line li-1
                                           // largest segment whose gen_col <= ours (segments are sorted by gen_col within a line)
    let i = segs.partition_point(|s| s.0 <= gen_col).checked_sub(1)?;
    let (col, line, src_col) = segs[i];
    let delta = (gen_col - col).saturating_sub(meta.leads[li - 1][i]);
    Some((line, src_col + delta))
}

type OrigPos = (u32, u32);
type OrigRange = (OrigPos, OrigPos, i64);

/// V8's covered ranges in original (line, col) coordinates; empty and unmappable ones dropped.
/// A range that opens before the first mapped generated position starts at (0, 0), and one that
/// closes after the last mapped position ends at the end of the source. That keeps the
/// module-wide ranges — the script and the CJS wrapper function, which open on wrapper line 0
/// and close past the source-map comment — and they are V8's only count for top-level code. A
/// runtime helper (it lies wholly before the first mapping) is still dropped.
fn orig_ranges(meta: &ScriptMeta, ranges: &[(usize, usize, i64)]) -> Vec<OrigRange> {
    let (first, last) = meta.code_span.unwrap_or((0, usize::MAX));
    let start = |o: usize| gen_to_orig(meta, o).or((o < first).then_some((0, 0)));
    let end = |o: usize| gen_to_orig(meta, o).or((o > last).then_some((u32::MAX, u32::MAX)));
    ranges
        .iter()
        .filter_map(|&(gs, ge, c)| Some((start(gs)?, end(ge)?, c)))
        .filter(|(s, e, _)| s < e)
        .collect()
}

/// The innermost range containing `p`: the latest start and, among equal starts, the earliest
/// end (a function at source (0, 0) starts where the module range does).
fn innermost(oranges: &[OrigRange], p: OrigPos) -> Option<OrigRange> {
    oranges
        .iter()
        .filter(|(s, e, _)| *s <= p && p < *e)
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .copied()
}

/// Branch coverage: map V8's generated block ranges into original (line, col) ranges, then look up
/// each AST decision arm's count to emit lcov BRDA/BRF/BRH.
pub fn map_branches(file: &Path, ranges: &[(usize, usize, i64)]) {
    let Some(meta) = meta_for(file) else { return };
    if meta.branches.is_empty() {
        return;
    }
    let oranges = orig_ranges(&meta, ranges);
    let count_at = |p: OrigPos| innermost(&oranges, p).map(|r| r.2).unwrap_or(0).max(0) as u64;
    for (block, br) in meta.branches.iter().enumerate() {
        let block_id = block as u32;
        let line = br.decision.0 + 1; // 1-based source line of the decision
        let block_count = count_at(br.decision);
        let reached = block_count > 0;
        if br.implicit_else && br.arms.len() == 2 {
            // `if` with no `else`: arms[0] = consequent start; the implicit else has no code of
            // its own, so its count is derived: else = (if-statement executions) − then.
            // The then-count is sampled at the consequent like any other arm (as vitest's v8
            // provider does). V8 keeps a consequent range only when its count differs from the
            // enclosing range's: a never-run consequent keeps a 0-count range, but one that ALWAYS
            // ran is dropped, so the enclosing count is then the then-count (then = block,
            // else = 0). No continuation search is needed (or safe: the first range after the
            // `if` may belong to a later nested function).
            let then_taken = count_at(br.arms[0]);
            let else_taken = block_count.saturating_sub(then_taken);
            record_branch(file, block_id, 0, line, reached, then_taken);
            record_branch(file, block_id, 1, line, reached, else_taken);
        } else {
            for (i, &arm) in br.arms.iter().enumerate() {
                record_branch(file, block_id, i as u32, line, reached, count_at(arm));
            }
        }
    }
}

/// Correlate each executable statement's original position with V8's covered byte ranges (mapped
/// gen→orig) to attribute a hit count, then record it. Mirrors `map_branches`; called alongside it.
pub fn map_statements(file: &Path, ranges: &[(usize, usize, i64)]) {
    let Some(meta) = meta_for(file) else { return };
    if meta.statements.is_empty() {
        return;
    }
    // the innermost range containing a statement's start gives its hit count
    let oranges = orig_ranges(&meta, ranges);
    let count_at = |p: OrigPos| innermost(&oranges, p).map(|r| r.2);
    for (idx, &pos) in meta.statements.iter().enumerate() {
        let line = pos.0 + 1; // 1-based source line of the statement
        let count = count_at(pos).unwrap_or(0).max(0) as u64;
        record_stmt(file, idx as u32, line, count);
    }
}

// ---- report emission ------------------------------------------------------------------------

/// Per-file coverage row used by every reporter + the gate.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    path: PathBuf,
    lh: u64,
    lf: u64,
    fnh: u64,
    fnf: u64,
    brh: u64,
    brf: u64,
    sh: u64,
    sf: u64,
}
impl Row {
    fn lpct(&self) -> f64 {
        if self.lf > 0 {
            100.0 * self.lh as f64 / self.lf as f64
        } else {
            100.0
        }
    }
    fn fpct(&self) -> f64 {
        if self.fnf > 0 {
            100.0 * self.fnh as f64 / self.fnf as f64
        } else {
            100.0
        }
    }
    fn bpct(&self) -> f64 {
        if self.brf > 0 {
            100.0 * self.brh as f64 / self.brf as f64
        } else {
            100.0
        }
    }
    fn spct(&self) -> f64 {
        if self.sf > 0 {
            100.0 * self.sh as f64 / self.sf as f64
        } else {
            100.0
        }
    }
}

/// Emit the configured coverage reporters and apply threshold gating. Returns `true` if the gate
/// passed (or no thresholds were configured), `false` if any threshold was unmet — the caller
/// turns a `false` into a non-zero process exit.
pub fn report() -> bool {
    let g = COV.lock().unwrap();
    let map = g.as_ref();
    // Zero instrumented source files is a hard FAILURE, not a vacuous 100% (0/0) green. Almost
    // always a misconfigured `coverage.include` (e.g. a brace glob that matched nothing) — a
    // silent pass there hides that the suite covers nothing. Distinguish "no source loaded at
    // all" from "include globs filtered everything out" so the message points at the cause.
    let nothing_collected = map.map(|m| m.is_empty()).unwrap_or(true);
    let mut files: Vec<(&PathBuf, &HashMap<u32, u64>)> = map
        .map(|m| {
            m.iter()
                // honor include/exclude globs + per-file ignore directive
                .filter(|(f, _)| passes_globs(f) && !is_ignored(f))
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        let inc = INCLUDE.lock().unwrap();
        eprint!("{}", zero_files_message(nothing_collected, &inc));
        return false;
    }
    files.sort_by(|a, b| a.0.cmp(b.0));

    let fn_g = FN_COV.lock().unwrap();
    let br_g = BR_COV.lock().unwrap();
    let st_g = ST_COV.lock().unwrap();
    let (lcov, rows, total) = build_rows(&files, fn_g.as_ref(), br_g.as_ref(), st_g.as_ref());

    let out_dir = out_dir();
    let _ = std::fs::create_dir_all(&out_dir);
    let reporters = reporters();
    let has = |r: &str| reporters.iter().any(|x| x == r);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // ---- lcov ----
    let lcov_path = out_dir.join("lcov.info");
    if has("lcov") {
        let _ = std::fs::write(&lcov_path, &lcov);
    }
    // ---- json-summary (vitest/c8 shape; no `statements` — V8 has no statement metric) ----
    if has("json-summary") || has("json") {
        let json_path = out_dir.join("coverage-summary.json");
        let _ = std::fs::write(&json_path, json_summary(&total, &rows));
    }
    // ---- html (browsable overview table) ----
    if has("html") {
        let html_path = out_dir.join("index.html");
        let _ = std::fs::write(&html_path, html_summary(&total, &rows, &cwd));
    }

    // ---- text (terminal summary) — on by default, suppress only if reporters set without it ----
    if has("text") {
        let mut outs: Vec<String> = Vec::new();
        if has("lcov") {
            outs.push(lcov_path.display().to_string());
        }
        if has("json-summary") || has("json") {
            outs.push(out_dir.join("coverage-summary.json").display().to_string());
        }
        if has("html") {
            outs.push(out_dir.join("index.html").display().to_string());
        }
        print!("{}", text_summary(&total, &rows, &cwd, &outs));
    }

    // ---- threshold gate ----
    gate(&total, &rows, &cwd)
}

/// Check global (and, under `--coverage-per-file`, per-file) thresholds. Prints each unmet metric
/// and returns `false` if any failed. No thresholds configured → always `true`.
fn gate(total: &Row, rows: &[Row], cwd: &Path) -> bool {
    let Some(t) = *THRESHOLDS.lock().unwrap() else {
        return true;
    };
    let failures = threshold_failures(&t, PER_FILE.load(Ordering::Relaxed), total, rows, cwd);
    print!("{}", gate_message(&failures));
    failures.is_empty()
}

/// Every unmet metric as `<label>: <metric> <pct>% < <min>%` — the total first, then (under
/// `per_file`) each file by its cwd-relative path.
fn threshold_failures(
    t: &Thresholds,
    per_file: bool,
    total: &Row,
    rows: &[Row],
    cwd: &Path,
) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();

    let check = |label: &str, fails: &mut Vec<String>, row: &Row| {
        let metrics: [(Option<f64>, &str, f64); 4] = [
            (t.lines, "lines", row.lpct()),
            (t.functions, "functions", row.fpct()),
            (t.branches, "branches", row.bpct()),
            (t.statements, "statements", row.spct()),
        ];
        for (thr, name, pct) in metrics {
            if let Some(min) = thr {
                if pct + 1e-9 < min {
                    fails.push(format!("{label}: {name} {pct:.2}% < {min}%"));
                }
            }
        }
    };

    check("total", &mut failures, total);
    if per_file {
        for r in rows {
            check(&short_path(&r.path, cwd), &mut failures, r);
        }
    }
    failures
}

/// The gate's stdout block: a pass line, or the numbered list of failures.
fn gate_message(failures: &[String]) -> String {
    if failures.is_empty() {
        return "  coverage thresholds met ✓\n".to_string();
    }
    let mut s = format!("\n  coverage threshold FAILURES ({}):\n", failures.len());
    for f in failures {
        s.push_str(&format!("    ✗ {f}\n"));
    }
    s
}

/// `path` relative to `cwd` when it lies under it, else the full path.
fn short_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// stderr explanation for the "0 instrumented files" hard failure.
fn zero_files_message(nothing_collected: bool, inc: &[String]) -> String {
    let mut s = String::from("\n  coverage ERROR: 0 source files instrumented.\n");
    if nothing_collected {
        s.push_str("    No instrumentable source was loaded under --coverage.\n");
    } else {
        s.push_str(&format!(
            "    The coverage.include globs matched no loaded source: [{}]\n",
            inc.join(", ")
        ));
    }
    s.push_str("    Refusing to report a vacuous 0/0 pass — check coverage.include / paths.\n");
    s
}

type LineHits = HashMap<u32, u64>;
type FnHits = HashMap<PathBuf, HashMap<(u32, String), u64>>;
type BrHits = HashMap<PathBuf, HashMap<(u32, u32), (u32, bool, u64)>>;
type StHits = HashMap<PathBuf, HashMap<u32, (u32, u64)>>;

/// Build the lcov text, the per-file rows and the totals row from the merged hit maps for the
/// (already filtered + sorted) `files`.
fn build_rows(
    files: &[(&PathBuf, &LineHits)],
    fn_map: Option<&FnHits>,
    br_map: Option<&BrHits>,
    st_map: Option<&StHits>,
) -> (String, Vec<Row>, Row) {
    let mut lcov = String::new();
    let (mut tot_lf, mut tot_lh, mut tot_fnf, mut tot_fnh, mut tot_brf, mut tot_brh) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut tot_sf, mut tot_sh) = (0u64, 0u64);
    let mut rows: Vec<Row> = Vec::new();
    for (file, lines) in files {
        lcov.push_str(&format!("SF:{}\n", file.display()));

        // functions (FN/FNDA), sorted by line then name
        let (mut fnf, mut fnh) = (0u64, 0u64);
        if let Some(fns) = fn_map.and_then(|m| m.get(*file)) {
            let mut fv: Vec<(&(u32, String), &u64)> = fns.iter().collect();
            fv.sort_by(|a, b| a.0 .0.cmp(&b.0 .0).then(a.0 .1.cmp(&b.0 .1)));
            for ((line, name), _) in &fv {
                lcov.push_str(&format!("FN:{},{}\n", line, name));
            }
            for ((_, name), c) in &fv {
                lcov.push_str(&format!("FNDA:{},{}\n", c, name));
            }
            fnf = fv.len() as u64;
            fnh = fv.iter().filter(|(_, c)| **c > 0).count() as u64;
            lcov.push_str(&format!("FNF:{fnf}\nFNH:{fnh}\n"));
        }
        tot_fnf += fnf;
        tot_fnh += fnh;

        // branches (BRDA), sorted by (block, branch)
        let (mut brf, mut brh) = (0u64, 0u64);
        if let Some(brs) = br_map.and_then(|m| m.get(*file)) {
            let mut bv: Vec<(&BranchKey, &BranchHit)> = brs.iter().collect();
            bv.sort_by(|a, b| a.0.cmp(b.0));
            for ((block, branch), (line, reached, taken)) in &bv {
                // lcov: taken is "-" when the containing block never executed.
                let t = if *reached {
                    taken.to_string()
                } else {
                    "-".to_string()
                };
                lcov.push_str(&format!("BRDA:{},{},{},{}\n", line, block, branch, t));
            }
            brf = bv.len() as u64;
            brh = bv
                .iter()
                .filter(|(_, (_, reached, taken))| *reached && *taken > 0)
                .count() as u64;
            lcov.push_str(&format!("BRF:{brf}\nBRH:{brh}\n"));
        }
        tot_brf += brf;
        tot_brh += brh;

        // lines (DA)
        let mut nums: Vec<(&u32, &u64)> = lines.iter().collect();
        nums.sort_by_key(|(l, _)| **l);
        let lf = nums.len() as u64;
        let lh = nums.iter().filter(|(_, c)| **c > 0).count() as u64;
        tot_lf += lf;
        tot_lh += lh;
        for (l, c) in &nums {
            lcov.push_str(&format!("DA:{},{}\n", l, c));
        }
        lcov.push_str(&format!("LF:{lf}\nLH:{lh}\nend_of_record\n"));

        // statements (no lcov field — counted for json-summary/text/html + the gate)
        let (mut sf, mut sh) = (0u64, 0u64);
        if let Some(sts) = st_map.and_then(|m| m.get(*file)) {
            sf = sts.len() as u64;
            sh = sts.values().filter(|(_, c)| *c > 0).count() as u64;
        }
        tot_sf += sf;
        tot_sh += sh;

        rows.push(Row {
            path: (*file).clone(),
            lh,
            lf,
            fnh,
            fnf,
            brh,
            brf,
            sh,
            sf,
        });
    }

    let total = Row {
        path: PathBuf::new(),
        lh: tot_lh,
        lf: tot_lf,
        fnh: tot_fnh,
        fnf: tot_fnf,
        brh: tot_brh,
        brf: tot_brf,
        sh: tot_sh,
        sf: tot_sf,
    };
    (lcov, rows, total)
}

/// The `text` reporter's terminal summary; `outs` = the report files written (may be empty).
fn text_summary(total: &Row, rows: &[Row], cwd: &Path, outs: &[String]) -> String {
    let mut s = format!(
        "\n Coverage — {} files (lines | funcs | branches | stmts)\n",
        rows.len()
    );
    for r in rows {
        s.push_str(&format!(
            "  {:>6.2}% ln  {:>6.2}% fn  {:>6.2}% br  {:>6.2}% st   {}\n",
            r.lpct(),
            r.fpct(),
            r.bpct(),
            r.spct(),
            short_path(&r.path, cwd)
        ));
    }
    s.push_str("  ------\n");
    s.push_str(&format!(
        "  {:>6.2}% lines ({}/{})   {:>6.2}% fns ({}/{})   {:>6.2}% branches ({}/{})   {:>6.2}% stmts ({}/{})\n",
        total.lpct(),
        total.lh,
        total.lf,
        total.fpct(),
        total.fnh,
        total.fnf,
        total.bpct(),
        total.brh,
        total.brf,
        total.spct(),
        total.sh,
        total.sf
    ));
    if !outs.is_empty() {
        s.push_str(&format!("  → {}\n", outs.join("  ")));
    }
    s
}

/// vitest/c8 `coverage-summary.json`: a `total` block + one block per absolute file path. Each
/// block has `lines`/`statements`/`functions`/`branches` with `{total,covered,skipped,pct}`.
fn json_summary(total: &Row, rows: &[Row]) -> String {
    fn metric(covered: u64, tot: u64, pct: f64) -> String {
        let pct = (pct * 100.0).round() / 100.0;
        format!(
            "{{\"total\":{},\"covered\":{},\"skipped\":0,\"pct\":{}}}",
            tot, covered, pct
        )
    }
    fn block(r: &Row) -> String {
        format!(
            "{{\"lines\":{},\"statements\":{},\"functions\":{},\"branches\":{}}}",
            metric(r.lh, r.lf, r.lpct()),
            metric(r.sh, r.sf, r.spct()),
            metric(r.fnh, r.fnf, r.fpct()),
            metric(r.brh, r.brf, r.bpct())
        )
    }
    // One entry per line, comma-separated (never a trailing comma, even with zero file rows).
    let mut entries = vec![format!("  \"total\": {}", block(total))];
    for r in rows {
        let key = r
            .path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        entries.push(format!("  \"{}\": {}", key, block(r)));
    }
    format!("{{\n{}\n}}\n", entries.join(",\n"))
}

/// Self-contained HTML overview table (c8/istanbul-style) — browsable `coverage/index.html`.
fn html_summary(total: &Row, rows: &[Row], cwd: &Path) -> String {
    fn cls(pct: f64) -> &'static str {
        if pct >= 90.0 {
            "high"
        } else if pct >= 75.0 {
            "med"
        } else {
            "low"
        }
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut body = String::new();
    let cell = |pct: f64, h: u64, f: u64| {
        format!(
            "<td class=\"{}\">{:.2}% <span class=\"frac\">({}/{})</span></td>",
            cls(pct),
            pct,
            h,
            f
        )
    };
    for r in rows {
        let short = short_path(&r.path, cwd);
        body.push_str(&format!(
            "<tr><td class=\"file\">{}</td>{}{}{}{}</tr>\n",
            esc(&short),
            cell(r.lpct(), r.lh, r.lf),
            cell(r.spct(), r.sh, r.sf),
            cell(r.fpct(), r.fnh, r.fnf),
            cell(r.bpct(), r.brh, r.brf)
        ));
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>turbo-test coverage</title><style>\
body{{font:14px/1.5 system-ui,sans-serif;margin:2rem;color:#222}}\
h1{{font-size:1.2rem}}table{{border-collapse:collapse;width:100%}}\
th,td{{padding:.35rem .6rem;border-bottom:1px solid #eee;text-align:right}}\
th:first-child,td.file{{text-align:left}}\
.frac{{color:#888;font-size:.85em}}\
.high{{color:#0a7d28}}.med{{color:#b8860b}}.low{{color:#c0392b}}\
tfoot td{{font-weight:600;border-top:2px solid #ccc}}\
</style></head><body><h1>Coverage — {} files</h1>\
<table><thead><tr><th>File</th><th>Lines</th><th>Statements</th><th>Functions</th><th>Branches</th></tr></thead>\
<tbody>\n{}</tbody><tfoot><tr><td class=\"file\">total</td>{}{}{}{}</tr></tfoot></table>\
</body></html>\n",
        rows.len(),
        body,
        cell(total.lpct(), total.lh, total.lf),
        cell(total.spct(), total.sh, total.sf),
        cell(total.fpct(), total.fnh, total.fnf),
        cell(total.bpct(), total.brh, total.brf)
    )
}

#[cfg(all(test, unix))]
#[path = "coverage_tests.rs"]
mod unit_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_globs_keeps_brace_commas_intact() {
        // top-level comma separates globs; commas inside {…} do NOT (the v0.2.7 bug).
        assert_eq!(split_globs("src/**/*.{ts,tsx}"), vec!["src/**/*.{ts,tsx}"]);
        assert_eq!(
            split_globs("src/**/*.ts, test/**/*.tsx"),
            vec!["src/**/*.ts", "test/**/*.tsx"]
        );
        assert_eq!(
            split_globs("src/**/*.{ts,tsx},lib/**/*.{js,jsx}"),
            vec!["src/**/*.{ts,tsx}", "lib/**/*.{js,jsx}"]
        );
    }

    #[test]
    fn brace_glob_matches_each_alternative() {
        assert!(glob_match("src/**/*.{ts,tsx}", "src/a/b.ts"));
        assert!(glob_match("src/**/*.{ts,tsx}", "src/a/b.tsx"));
        assert!(!glob_match("src/**/*.{ts,tsx}", "src/a/b.js"));
        // ** spans separators; * does not
        assert!(glob_match("src/**", "src/a/b/c.ts"));
        assert!(glob_match("src/*.ts", "src/x.ts"));
        assert!(!glob_match("src/*.ts", "src/a/x.ts"));
    }
}
