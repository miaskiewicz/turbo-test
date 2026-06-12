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
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("coverage")
}

// ---- report config: thresholds, reporters, include/exclude (M6 coverage gating) -------------
//
// `statements` is derived (V8 has no native statement counter): the original source is parsed
// with oxc, and each executable statement's position is correlated with V8's covered byte ranges
// — the same machinery as branches. This mirrors how c8 reports statements, so vitest/c8 users
// get the metric they expect. (lcov has no statement field, so it appears in json-summary / text
// / html only.)

// Per-metric minimum percentages (0..=100). None = not gated on that metric.
#[derive(Clone, Copy, Default)]
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
    let mut t = THRESHOLDS.lock().unwrap().unwrap_or_default();
    for part in spec.split(',') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let Ok(n) = v.trim().parse::<f64>() else { continue };
        match k.trim() {
            "lines" => t.lines = Some(n),
            "functions" | "funcs" => t.functions = Some(n),
            "branches" => t.branches = Some(n),
            "statements" | "stmts" => t.statements = Some(n),
            _ => {}
        }
    }
    *THRESHOLDS.lock().unwrap() = Some(t);
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
    REPORTERS
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| vec!["lcov".to_string(), "json-summary".to_string(), "text".to_string()])
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
    IGNORED.lock().unwrap().get_or_insert_with(HashSet::new).insert(file.to_path_buf());
}
fn is_ignored(file: &Path) -> bool {
    IGNORED.lock().unwrap().as_ref().map(|s| s.contains(file)).unwrap_or(false)
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
    expand_braces(pat).iter().any(|p| simple_match(p.as_bytes(), path.as_bytes()))
}

fn expand_braces(pat: &str) -> Vec<String> {
    let Some(open) = pat.find('{') else { return vec![pat.to_string()] };
    let Some(rel_close) = pat[open..].find('}') else { return vec![pat.to_string()] };
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

/// Whether a file passes the configured include/exclude globs. Matched against the file's
/// cwd-relative POSIX path (how vitest interprets `coverage.include`).
fn passes_globs(file: &Path) -> bool {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let rel = file.strip_prefix(&cwd).unwrap_or(file);
    let rel = rel.to_string_lossy().replace('\\', "/");
    let inc = INCLUDE.lock().unwrap();
    let exc = EXCLUDE.lock().unwrap();
    if !inc.is_empty() && !inc.iter().any(|g| glob_match(g, &rel)) {
        return false;
    }
    if exc.iter().any(|g| glob_match(g, &rel)) {
        return false;
    }
    true
}

// file -> (origLine -> max hit count). Presence of a line = it's executable; value > 0 = covered.
static COV: Mutex<Option<HashMap<PathBuf, HashMap<u32, u64>>>> = Mutex::new(None);
// file -> ((origLine, fnName) -> max call count). Function coverage.
static FN_COV: Mutex<Option<HashMap<PathBuf, HashMap<(u32, String), u64>>>> = Mutex::new(None);
// file -> ((block, branch) -> (line, reached, max taken)). Branch coverage.
static BR_COV: Mutex<Option<HashMap<PathBuf, HashMap<(u32, u32), (u32, bool, u64)>>>> =
    Mutex::new(None);
// file -> (statement index -> (line, max hit count)). Statement coverage.
static ST_COV: Mutex<Option<HashMap<PathBuf, HashMap<u32, (u32, u64)>>>> = Mutex::new(None);

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
    line_start: Vec<usize>,        // UTF-16 line starts of the WRAPPED compiled source
    genmap: HashMap<u32, u32>,     // gen line -> src line (first segment) — line coverage
    segments: Vec<Vec<(u32, u32, u32)>>, // per gen line: (gen_col, src_line, src_col) — gen→orig
    branches: Vec<BranchCC>,       // decision points in original (line, col)
    statements: Vec<(u32, u32)>,   // executable statement starts in original (line, col)
}
static META: Mutex<Option<HashMap<PathBuf, Option<Arc<ScriptMeta>>>>> = Mutex::new(None);

pub fn has_meta(file: &Path) -> bool {
    META.lock().unwrap().as_ref().map(|m| m.contains_key(file)).unwrap_or(false)
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
                arms: b.arms.iter().map(|&o| byte_to_linecol(orig_source, o)).collect(),
                implicit_else: b.implicit_else,
            })
            .collect();
        let statements = raw_stmts
            .into_iter()
            .map(|o| byte_to_linecol(orig_source, o))
            .collect();
        Arc::new(ScriptMeta { line_start, genmap, segments, branches, statements })
    });
    let mut g = META.lock().unwrap();
    g.get_or_insert_with(HashMap::new).insert(file.to_path_buf(), meta);
}

fn meta_for(file: &Path) -> Option<Arc<ScriptMeta>> {
    META.lock().unwrap().as_ref().and_then(|m| m.get(file).cloned()).flatten()
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
    fn send_response(
        &self,
        _call_id: i32,
        message: v8::UniquePtr<v8::inspector::StringBuffer>,
    ) {
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
        Collector { inspector, session: None, resp: Rc::new(RefCell::new(None)) }
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
        let channel = v8::inspector::Channel::new(Box::new(CovChannel { resp: self.resp.clone() }));
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
        dispatch(session, br#"{"id":3,"method":"Profiler.takePreciseCoverage"}"#);
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

    // map a byte offset → original source line (via wrapped line → esbuild gen line → src line).
    let off_to_src = |off: usize| -> Option<u32> {
        // largest line index whose start is <= off
        let li = line_start.partition_point(|&s| s <= off).saturating_sub(1);
        if li == 0 {
            return None; // wrapper line
        }
        genmap.get(&((li - 1) as u32)).map(|&sl| sl + 1)
    };

    // innermost covered range for an offset = the one with the largest start that still contains it.
    let innermost = |off: usize| -> Option<i64> {
        let mut best: Option<(usize, i64)> = None;
        for &(s, e, c) in ranges {
            if s <= off && off < e {
                if best.map(|(bs, _)| s >= bs).unwrap_or(true) {
                    best = Some((s, c));
                }
            }
        }
        best.map(|(_, c)| c)
    };

    // line coverage
    for (li, &off) in line_start.iter().enumerate() {
        if li == 0 {
            continue; // wrapper line
        }
        let Some(count) = innermost(off) else { continue };
        if let Some(src_line) = off_to_src(off) {
            record(file, src_line, count.max(0) as u64);
        }
    }

    // function coverage — each function's outer range start offset → its declaration line.
    for (name, start, count) in funcs {
        if let Some(src_line) = off_to_src(*start) {
            let n = if name.is_empty() { "(anonymous)" } else { name.as_str() };
            record_fn(file, src_line, n, (*count).max(0) as u64);
        }
    }
}

/// A WRAPPED generated byte offset → original (src_line, src_col), both 0-based, via the source map
/// segments. Within a segment the offset delta is applied 1:1 (esbuild segments are identity runs).
fn gen_to_orig(meta: &ScriptMeta, off: usize) -> Option<(u32, u32)> {
    let idx = meta.line_start.partition_point(|&s| s <= off);
    if idx == 0 {
        return None;
    }
    let li = idx - 1; // 0-based wrapped line
    if li == 0 {
        return None; // wrapper line
    }
    let gen_col = (off - meta.line_start[li]) as u32;
    let segs = meta.segments.get(li - 1)?; // wrapped line li → esbuild gen line li-1
    if segs.is_empty() {
        return None;
    }
    // largest segment whose gen_col <= ours (segments are sorted by gen_col within a line)
    let mut chosen: Option<&(u32, u32, u32)> = None;
    for s in segs {
        if s.0 <= gen_col {
            chosen = Some(s);
        } else {
            break;
        }
    }
    let s = chosen?;
    Some((s.1, s.2 + (gen_col - s.0)))
}

/// Branch coverage: map V8's generated block ranges into original (line, col) ranges, then look up
/// each AST decision arm's count to emit lcov BRDA/BRF/BRH.
pub fn map_branches(file: &Path, ranges: &[(usize, usize, i64)]) {
    let Some(meta) = meta_for(file) else { return };
    if meta.branches.is_empty() {
        return;
    }
    // original-coordinate covered ranges
    let mut oranges: Vec<((u32, u32), (u32, u32), i64)> = Vec::new();
    for &(gs, ge, c) in ranges {
        if let (Some(s), Some(e)) = (gen_to_orig(&meta, gs), gen_to_orig(&meta, ge)) {
            if s < e {
                oranges.push((s, e, c));
            }
        }
    }
    let count_at = |p: (u32, u32)| -> Option<i64> {
        let mut best: Option<((u32, u32), i64)> = None;
        for &(s, e, c) in &oranges {
            if s <= p && p < e && best.map(|(bs, _)| s >= bs).unwrap_or(true) {
                best = Some((s, c));
            }
        }
        best.map(|(_, c)| c)
    };
    // The innermost covered range CONTAINING `p` (start, end, count) — gives the enclosing block.
    let block_of = |p: (u32, u32)| -> Option<((u32, u32), (u32, u32), i64)> {
        let mut best: Option<((u32, u32), (u32, u32), i64)> = None;
        for &(s, e, c) in &oranges {
            if s <= p && p < e && best.map(|(bs, _, _)| s >= bs).unwrap_or(true) {
                best = Some((s, e, c));
            }
        }
        best
    };
    // The covered range with the SMALLEST start within `[lo, hi)` — used to find a consequent's
    // own block, which V8 may start a column or two past the AST keyword position (so plain
    // point-containment at the keyword misses it and falls through to the parent count).
    let first_range_in = |lo: (u32, u32), hi: (u32, u32)| -> Option<i64> {
        oranges
            .iter()
            .filter(|(s, _, _)| *s >= lo && *s < hi)
            .min_by_key(|(s, _, _)| *s)
            .map(|(_, _, c)| *c)
    };
    for (block, br) in meta.branches.iter().enumerate() {
        let block_id = block as u32;
        let line = br.decision.0 + 1; // 1-based source line of the decision
        let block_count = count_at(br.decision).unwrap_or(0).max(0) as u64;
        let reached = block_count > 0;
        if br.implicit_else && br.arms.len() == 2 {
            // `if` with no `else`. arms[0] = consequent start, arms[1] = if-statement end (the
            // implicit-else position). Derive then/else from V8's sub-ranges rather than a single
            // point sample — robust to the braceless `if (c) return X;` shape where V8 starts the
            // consequent's range a column past the `return`/`throw` keyword.
            let (then_taken, else_taken) =
                if let Some(c) = first_range_in(br.arms[0], br.arms[1]) {
                    // consequent has its own block (then taken sometimes-but-not-always): that
                    // range's count IS the then-count; else = block − then.
                    let t = c.max(0) as u64;
                    (t, block_count.saturating_sub(t))
                } else {
                    // No distinct consequent range. Two cases, told apart by the continuation:
                    //  - then ALWAYS taken → code after the `if` is unreachable → V8 emits a
                    //    lower-count continuation range → else = that count (0), then = block.
                    //  - then NEVER taken → continuation count == block (no distinct range) →
                    //    then = 0, else = block.
                    let block_end = block_of(br.decision).map(|(_, e, _)| e).unwrap_or((u32::MAX, 0));
                    match first_range_in(br.arms[1], block_end) {
                        Some(c) => {
                            let e = c.max(0) as u64;
                            (block_count.saturating_sub(e), e)
                        }
                        None => (0, block_count),
                    }
                };
            record_branch(file, block_id, 0, line, reached, then_taken);
            record_branch(file, block_id, 1, line, reached, else_taken);
        } else {
            for (i, &arm) in br.arms.iter().enumerate() {
                let taken = count_at(arm).unwrap_or(0).max(0) as u64;
                record_branch(file, block_id, i as u32, line, reached, taken);
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
    // original-coordinate covered ranges (same construction as map_branches)
    let mut oranges: Vec<((u32, u32), (u32, u32), i64)> = Vec::new();
    for &(gs, ge, c) in ranges {
        if let (Some(s), Some(e)) = (gen_to_orig(&meta, gs), gen_to_orig(&meta, ge)) {
            if s < e {
                oranges.push((s, e, c));
            }
        }
    }
    // innermost (smallest-start) range containing `p` gives that statement's hit count.
    let count_at = |p: (u32, u32)| -> Option<i64> {
        let mut best: Option<((u32, u32), i64)> = None;
        for &(s, e, c) in &oranges {
            if s <= p && p < e && best.map(|(bs, _)| s >= bs).unwrap_or(true) {
                best = Some((s, c));
            }
        }
        best.map(|(_, c)| c)
    };
    for (idx, &pos) in meta.statements.iter().enumerate() {
        let line = pos.0 + 1; // 1-based source line of the statement
        let count = count_at(pos).unwrap_or(0).max(0) as u64;
        record_stmt(file, idx as u32, line, count);
    }
}

// ---- report emission ------------------------------------------------------------------------

/// Per-file coverage row used by every reporter + the gate.
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
        if self.lf > 0 { 100.0 * self.lh as f64 / self.lf as f64 } else { 100.0 }
    }
    fn fpct(&self) -> f64 {
        if self.fnf > 0 { 100.0 * self.fnh as f64 / self.fnf as f64 } else { 100.0 }
    }
    fn bpct(&self) -> f64 {
        if self.brf > 0 { 100.0 * self.brh as f64 / self.brf as f64 } else { 100.0 }
    }
    fn spct(&self) -> f64 {
        if self.sf > 0 { 100.0 * self.sh as f64 / self.sf as f64 } else { 100.0 }
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
        eprintln!("\n  coverage ERROR: 0 source files instrumented.");
        if nothing_collected {
            eprintln!("    No instrumentable source was loaded under --coverage.");
        } else {
            let inc = INCLUDE.lock().unwrap();
            eprintln!(
                "    The coverage.include globs matched no loaded source: [{}]",
                inc.join(", ")
            );
        }
        eprintln!("    Refusing to report a vacuous 0/0 pass — check coverage.include / paths.");
        return false;
    }
    files.sort_by(|a, b| a.0.cmp(b.0));

    let fn_g = FN_COV.lock().unwrap();
    let fn_map = fn_g.as_ref();
    let br_g = BR_COV.lock().unwrap();
    let br_map = br_g.as_ref();
    let st_g = ST_COV.lock().unwrap();
    let st_map = st_g.as_ref();

    let mut lcov = String::new();
    let (mut tot_lf, mut tot_lh, mut tot_fnf, mut tot_fnh, mut tot_brf, mut tot_brh) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut tot_sf, mut tot_sh) = (0u64, 0u64);
    let mut rows: Vec<Row> = Vec::new();
    for (file, lines) in &files {
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
            let mut bv: Vec<(&(u32, u32), &(u32, bool, u64))> = brs.iter().collect();
            bv.sort_by(|a, b| a.0.cmp(b.0));
            for ((block, branch), (line, reached, taken)) in &bv {
                // lcov: taken is "-" when the containing block never executed.
                let t = if *reached { taken.to_string() } else { "-".to_string() };
                lcov.push_str(&format!("BRDA:{},{},{},{}\n", line, block, branch, t));
            }
            brf = bv.len() as u64;
            brh = bv.iter().filter(|(_, (_, reached, taken))| *reached && *taken > 0).count() as u64;
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

        rows.push(Row { path: (*file).clone(), lh, lf, fnh, fnf, brh, brf, sh, sf });
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
        println!("\n Coverage — {} files (lines | funcs | branches | stmts)", rows.len());
        for r in &rows {
            let short = r
                .path
                .strip_prefix(&cwd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| r.path.to_string_lossy().into_owned());
            println!(
                "  {:>6.2}% ln  {:>6.2}% fn  {:>6.2}% br  {:>6.2}% st   {}",
                r.lpct(),
                r.fpct(),
                r.bpct(),
                r.spct(),
                short
            );
        }
        println!("  ------");
        println!(
            "  {:>6.2}% lines ({}/{})   {:>6.2}% fns ({}/{})   {:>6.2}% branches ({}/{})   {:>6.2}% stmts ({}/{})",
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
        );
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
        if !outs.is_empty() {
            println!("  → {}", outs.join("  "));
        }
    }

    // ---- threshold gate ----
    gate(&total, &rows, &cwd)
}

/// Check global (and, under `--coverage-per-file`, per-file) thresholds. Prints each unmet metric
/// and returns `false` if any failed. No thresholds configured → always `true`.
fn gate(total: &Row, rows: &[Row], cwd: &Path) -> bool {
    let Some(t) = *THRESHOLDS.lock().unwrap() else { return true };
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
    if PER_FILE.load(Ordering::Relaxed) {
        for r in rows {
            let short = r
                .path
                .strip_prefix(cwd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| r.path.to_string_lossy().into_owned());
            check(&short, &mut failures, r);
        }
    }

    if failures.is_empty() {
        println!("  coverage thresholds met ✓");
        return true;
    }
    println!("\n  coverage threshold FAILURES ({}):", failures.len());
    for f in &failures {
        println!("    ✗ {f}");
    }
    false
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
    let mut s = String::from("{\n");
    s.push_str(&format!("  \"total\": {},\n", block(total)));
    for (i, r) in rows.iter().enumerate() {
        let key = r.path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let comma = if i + 1 < rows.len() { "," } else { "" };
        s.push_str(&format!("  \"{}\": {}{}\n", key, block(r), comma));
    }
    s.push_str("}\n");
    s
}

/// Self-contained HTML overview table (c8/istanbul-style) — browsable `coverage/index.html`.
fn html_summary(total: &Row, rows: &[Row], cwd: &Path) -> String {
    fn cls(pct: f64) -> &'static str {
        if pct >= 90.0 { "high" } else if pct >= 75.0 { "med" } else { "low" }
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
        let short = r
            .path
            .strip_prefix(cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| r.path.to_string_lossy().into_owned());
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
