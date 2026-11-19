//! Host process figures behind process.uptime / memoryUsage / cpuUsage / resourceUsage / umask
//! (turbo-test#419), in the units Node reports them: getrusage times in microseconds, maxRSS in
//! kilobytes, the resident set and V8 heap sizes in bytes. The JS side (node_process_usage.js)
//! shapes them into Node's objects and validates arguments.
use std::sync::LazyLock;
use std::time::Instant;

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Record the process start for process.uptime(); later calls keep the first instant.
pub fn mark_start() {
    LazyLock::force(&START);
}

fn set_numbers(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, values: &[f64]) {
    let elements: Vec<v8::Local<v8::Value>> = values
        .iter()
        .map(|value| v8::Number::new(scope, *value).into())
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &elements).into());
}

/// native __ttUptime(): seconds since the process started
pub fn native_uptime(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set_double(START.elapsed().as_secs_f64());
}

/// native __ttHeapUsage(): [heapTotal, heapUsed, external] of this isolate's V8 heap. V8 counts
/// ArrayBuffer backing stores in its external memory, so `external` also bounds `arrayBuffers`.
pub fn native_heap_usage(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let stats = scope.get_heap_statistics();
    let values = [
        stats.total_heap_size() as f64,
        stats.used_heap_size() as f64,
        stats.external_memory() as f64,
    ];
    set_numbers(scope, &mut rv, &values);
}

/// The process's resident set in bytes (None when the OS does not report it).
#[cfg(target_os = "macos")]
pub(crate) fn resident_set_size() -> Option<f64> {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let pid = std::process::id() as libc::c_int;
    let info_ptr = (&mut info as *mut libc::proc_taskinfo).cast();
    let read = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, info_ptr, size) };
    (read == size).then_some(info.pti_resident_size as f64)
}

#[cfg(target_os = "linux")]
pub(crate) fn resident_set_size() -> Option<f64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn resident_set_size() -> Option<f64> {
    None
}

/// native __ttResidentSetSize(): the resident set in bytes, or undefined when unknown
pub fn native_resident_set_size(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if let Some(bytes) = resident_set_size() {
        rv.set_double(bytes);
    }
}

#[cfg(unix)]
fn rusage() -> libc::rusage {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage
}

#[cfg(unix)]
fn micros(time: libc::timeval) -> f64 {
    time.tv_sec as f64 * 1e6 + time.tv_usec as f64
}

/// getrusage's ru_maxrss unit per kilobyte: bytes on macOS, kilobytes elsewhere (libuv)
#[cfg(all(unix, target_os = "macos"))]
const MAXRSS_PER_KB: f64 = 1024.0;
#[cfg(all(unix, not(target_os = "macos")))]
const MAXRSS_PER_KB: f64 = 1.0;

/// Node's resourceUsage fields, in its order
#[cfg(unix)]
pub(crate) fn resource_usage(usage: &libc::rusage) -> [f64; 16] {
    [
        micros(usage.ru_utime),
        micros(usage.ru_stime),
        (usage.ru_maxrss as f64 / MAXRSS_PER_KB).floor(),
        usage.ru_ixrss as f64,
        usage.ru_idrss as f64,
        usage.ru_isrss as f64,
        usage.ru_minflt as f64,
        usage.ru_majflt as f64,
        usage.ru_nswap as f64,
        usage.ru_inblock as f64,
        usage.ru_oublock as f64,
        usage.ru_msgsnd as f64,
        usage.ru_msgrcv as f64,
        usage.ru_nsignals as f64,
        usage.ru_nvcsw as f64,
        usage.ru_nivcsw as f64,
    ]
}

/// native __ttCpuUsage(): [user, system] CPU time of the whole process in microseconds
#[cfg(unix)]
pub fn native_cpu_usage(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let usage = rusage();
    set_numbers(
        scope,
        &mut rv,
        &[micros(usage.ru_utime), micros(usage.ru_stime)],
    );
}

/// native __ttResourceUsage(): resource_usage of the whole process
#[cfg(unix)]
pub fn native_resource_usage(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    set_numbers(scope, &mut rv, &resource_usage(&rusage()));
}

/// native __ttUmask(mask?): set the file mode creation mask (a validated uint32 from the JS
/// side) and return the previous one; without a mask, read it as Node does (set 0, restore).
#[cfg(unix)]
pub fn native_umask(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let mask = args.get(0);
    let previous = if mask.is_undefined() {
        let current = unsafe { libc::umask(0) };
        unsafe { libc::umask(current) };
        current
    } else {
        unsafe { libc::umask(mask.uint32_value(scope).unwrap_or(0) as libc::mode_t) }
    };
    rv.set_double(f64::from(previous));
}
