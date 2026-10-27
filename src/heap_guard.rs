//! Per-isolate heap guard: turn "V8 aborts the whole process at the heap limit" into "this file
//! fails and the worker gets a new isolate".
//!
//! Without it, V8's default response to an exhausted heap is `FatalProcessOutOfMemory`, which kills
//! every worker and loses the results of every file already run (miaskiewicz/turbo-test#79: a
//! 529-file suite under isolate reuse died at 2.0 GB with no per-file output at all; tracked as
//! turbo-test#124). From alphajew420's miaskiewicz/turbo-test#104.
use std::cell::Cell;
use std::ffi::c_void;

thread_local! {
    /// Set by the near-heap-limit callback on this worker thread; consumed by `take_tripped`.
    static TRIPPED: Cell<bool> = const { Cell::new(false) };
    static GRANTS: Cell<u32> = const { Cell::new(0) };
}

/// Extra headroom granted to an isolate that hit its limit so termination can unwind it.
const GRANT_BYTES: usize = 192 << 20;
/// A callback that keeps being re-entered means the script is not unwinding; give up (V8 aborts).
const MAX_GRANTS: u32 = 4;

unsafe extern "C" fn near_heap_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    let handle = unsafe { &*(data as *const v8::IsolateHandle) };
    TRIPPED.with(|t| t.set(true));
    handle.terminate_execution();
    let grants = GRANTS.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    if grants > MAX_GRANTS {
        current
    } else {
        current + GRANT_BYTES
    }
}

/// The callback's data: owned by the isolate (a slot), so it lives exactly as long as the isolate
/// that holds the raw pointer and is dropped with it (fresh mode creates an isolate per file).
struct GuardHandle(#[allow(dead_code)] Box<v8::IsolateHandle>);

/// Register the guard on a new isolate.
pub fn install(isolate: &mut v8::Isolate) {
    let handle = Box::new(isolate.thread_safe_handle());
    let data = &*handle as *const v8::IsolateHandle as *mut c_void;
    isolate.set_slot(GuardHandle(handle));
    isolate.add_near_heap_limit_callback(near_heap_limit, data);
}

/// Start of a file: forget a previous file's trip.
pub fn arm() {
    TRIPPED.with(|t| t.set(false));
    GRANTS.with(|g| g.set(0));
}

/// Whether the isolate ran into its heap limit since `arm`.
pub fn tripped() -> bool {
    TRIPPED.with(|t| t.get())
}

/// `TURBO_MAX_HEAP_MB`: hard heap limit per isolate (V8's default when unset).
pub fn max_heap_bytes() -> Option<usize> {
    std::env::var("TURBO_MAX_HEAP_MB")
        .ok()?
        .parse::<usize>()
        .ok()
        .map(|mb| mb << 20)
}

/// Live-heap size above which a reused isolate is replaced after its file.
/// `TURBO_HEAP_RECYCLE_MB`, else 60% of the isolate's heap limit.
pub fn recycle_threshold(limit: usize) -> usize {
    std::env::var("TURBO_HEAP_RECYCLE_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|mb| mb << 20)
        .unwrap_or(limit / 10 * 6)
}

/// `TURBO_HEAP_LOG`: one stderr line per file with the isolate's heap after a full GC.
pub fn log_enabled() -> bool {
    std::env::var_os("TURBO_HEAP_LOG").is_some_and(|v| !v.is_empty() && v != "0")
}

pub fn mb(bytes: usize) -> usize {
    bytes >> 20
}

/// Start of the error a file that ran out of heap is reported with.
pub const OOM_PREFIX: &str = "JavaScript heap out of memory";

pub fn oom_message(limit: usize) -> String {
    format!(
        "{OOM_PREFIX} (limit {} MB): the file was aborted and its isolate discarded",
        mb(limit)
    )
}

/// Whether a file error is the heap guard's: not transient (re-running the file on the same
/// kind of isolate would only run out of memory again).
pub fn is_oom(err: &str) -> bool {
    err.starts_with(OOM_PREFIX)
}
