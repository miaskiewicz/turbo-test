//! Persistent node worker that keeps `require("typescript")` loaded across many
//! decorator-metadata transforms (turbo-test#431). The one-shot `node -e <inline>`
//! path still lives in `runner::spawn_tsc_stdin`; this module is the reusable
//! alternative the loader prefers. This stub is replaced by the real worker in the
//! follow-up fix commit — the failing tests here describe what that worker must do.

#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SPAWN_COUNT: AtomicU64 = AtomicU64::new(0);

/// How many persistent workers this process has spawned since start. One run that
/// transforms many files for the same project must not exceed 1.
pub fn spawn_count() -> u64 {
    SPAWN_COUNT.load(Ordering::Relaxed)
}

/// Transform one TS file with the project's TypeScript compiler through the persistent
/// worker. The stub always returns `None` so the caller keeps using the one-shot path
/// until the worker lands.
pub fn transform(
    _root: &Path,
    _ts_dir: &Path,
    _file: &Path,
    _module_kind: &str,
    _src: &str,
) -> Option<String> {
    None
}

#[cfg(test)]
#[path = "tsc_worker_tests.rs"]
mod tests;
