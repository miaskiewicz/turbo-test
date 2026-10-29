//! turbo-test — consolidated native test runner CLI (M1 end-to-end + M4 parallelism).
//!
//! Runs test files across N worker threads, each with its own V8 isolate booted from the
//! single shared framework snapshot (spec §4). Work-stealing via an atomic cursor balances
//! load; files are ordered slowest-first from persisted historical durations (duration-aware
//! scheduling) so the longest files start first and workers drain evenly. Results are
//! returned in-process (no IPC serialization needed for the thread model).
//!
//! The implementation lives in `turbo_test::cli` (argument parsing, the worker pool, the
//! reporters) and `turbo_test::launcher` (the vitest-style front-end); this binary only forwards
//! argv and exits with the returned code.

fn main() {
    std::process::exit(turbo_test::cli::main_with_args(
        std::env::args().skip(1).collect(),
    ));
}
