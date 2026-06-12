//! Native code coverage via V8's Inspector `Profiler` precise-coverage domain.
//!
//! Everything here is gated behind `enabled()` (the `--coverage` flag) — when off, the runner
//! never constructs a Collector, names a script, or emits a source map, so the default fast path
//! is byte-identical. V8 precise coverage is near-zero runtime overhead (no Istanbul-style
//! instrumentation): the engine records per-function call counts and block ranges internally, we
//! just read them out at the end of each file and remap the byte ranges back to original `.ts`
//! lines through the esbuild source map we emit (under coverage only).

use std::cell::RefCell;
use std::collections::HashMap;
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

// file -> (origLine -> max hit count). Presence of a line = it's executable; value > 0 = covered.
static COV: Mutex<Option<HashMap<PathBuf, HashMap<u32, u64>>>> = Mutex::new(None);
// file -> ((origLine, fnName) -> max call count). Function coverage.
static FN_COV: Mutex<Option<HashMap<PathBuf, HashMap<(u32, String), u64>>>> = Mutex::new(None);

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
struct ScriptMeta {
    line_start: Vec<usize>,
    genmap: HashMap<u32, u32>,
}
static META: Mutex<Option<HashMap<PathBuf, Option<Arc<ScriptMeta>>>>> = Mutex::new(None);

pub fn has_meta(file: &Path) -> bool {
    META.lock().unwrap().as_ref().map(|m| m.contains_key(file)).unwrap_or(false)
}

/// Build + cache the static map data for a source file from its wrapped compiled source. Stores
/// `None` for files whose source map can't be parsed (so we don't retry them).
pub fn register_meta(file: &Path, wrapped: &str) {
    let meta = inline_map_genline_to_srcline(wrapped).map(|genmap| {
        let u16s: Vec<u16> = wrapped.encode_utf16().collect();
        let mut line_start = vec![0usize];
        for (i, &c) in u16s.iter().enumerate() {
            if c == 0x0A {
                line_start.push(i + 1);
            }
        }
        Arc::new(ScriptMeta { line_start, genmap })
    });
    let mut g = META.lock().unwrap();
    g.get_or_insert_with(HashMap::new).insert(file.to_path_buf(), meta);
}

fn meta_for(file: &Path) -> Option<Arc<ScriptMeta>> {
    META.lock().unwrap().as_ref().and_then(|m| m.get(file).cloned()).flatten()
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
fn inline_map_genline_to_srcline(transformed: &str) -> Option<HashMap<u32, u32>> {
    let marker = "//# sourceMappingURL=data:application/json;base64,";
    let pos = transformed.rfind(marker)?;
    let b64 = transformed[pos + marker.len()..].lines().next()?.trim();
    let json = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&json).ok()?;
    let mappings = v.get("mappings")?.as_str()?;
    Some(parse_mappings(mappings))
}

fn parse_mappings(mappings: &str) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    // src_line/col/idx are cumulative across the WHOLE mappings string; gen_col resets per line.
    let (mut src_idx, mut src_line, mut src_col) = (0i64, 0i64, 0i64);
    for (gen_line, line) in mappings.split(';').enumerate() {
        let mut first: Option<i64> = None;
        for seg in line.split(',') {
            if seg.is_empty() {
                continue;
            }
            let vals = vlq_decode(seg);
            if vals.len() >= 4 {
                src_idx += vals[1];
                src_line += vals[2];
                src_col += vals[3];
                let _ = (src_idx, src_col);
                if first.is_none() {
                    first = Some(src_line);
                }
            }
        }
        if let Some(sl) = first {
            if sl >= 0 {
                out.insert(gen_line as u32, sl as u32);
            }
        }
    }
    out
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

// ---- report emission ------------------------------------------------------------------------

/// Write `coverage/lcov.info` and print a per-file + total line-coverage summary. Returns the
/// (covered, total) line counts for the run.
pub fn report() -> (u64, u64) {
    let g = COV.lock().unwrap();
    let Some(map) = g.as_ref() else {
        println!("\ncoverage: no data collected.");
        return (0, 0);
    };
    let mut files: Vec<(&PathBuf, &HashMap<u32, u64>)> = map.iter().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));

    let fn_g = FN_COV.lock().unwrap();
    let fn_map = fn_g.as_ref();

    let mut lcov = String::new();
    let (mut tot_lf, mut tot_lh, mut tot_fnf, mut tot_fnh) = (0u64, 0u64, 0u64, 0u64);
    // row: (path, lh, lf, fnh, fnf)
    let mut rows: Vec<(String, u64, u64, u64, u64)> = Vec::new();
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
        rows.push((file.to_string_lossy().into_owned(), lh, lf, fnh, fnf));
    }

    let out_dir = out_dir();
    let _ = std::fs::create_dir_all(&out_dir);
    let lcov_path = out_dir.join("lcov.info");
    let _ = std::fs::write(&lcov_path, &lcov);

    // shorten paths against cwd for the summary
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("\n Coverage — {} files (lines | funcs)", rows.len());
    for (f, lh, lf, fnh, fnf) in &rows {
        let lpct = if *lf > 0 { 100.0 * *lh as f64 / *lf as f64 } else { 100.0 };
        let fpct = if *fnf > 0 { 100.0 * *fnh as f64 / *fnf as f64 } else { 100.0 };
        let short = Path::new(f).strip_prefix(&cwd).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| f.clone());
        println!("  {:>6.2}% lines  {:>6.2}% fns   {}", lpct, fpct, short);
    }
    let ltot = if tot_lf > 0 { 100.0 * tot_lh as f64 / tot_lf as f64 } else { 100.0 };
    let ftot = if tot_fnf > 0 { 100.0 * tot_fnh as f64 / tot_fnf as f64 } else { 100.0 };
    println!("  ------");
    println!(
        "  {:>6.2}% lines ({}/{})   {:>6.2}% fns ({}/{})   → {}",
        ltot, tot_lh, tot_lf, ftot, tot_fnh, tot_fnf, lcov_path.display()
    );
    (tot_lh, tot_lf)
}
