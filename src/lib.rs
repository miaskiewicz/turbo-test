//! turbo-test — native test runner internals (below the user-observable line).
//!
//! Shared substrate used by the milestone spikes and (eventually) the consolidated
//! ModuleRunner. Public surface is deliberately small: plumbing, not framework.

pub mod bundler;
pub mod coverage;
pub mod coverage_branch;
pub mod esm_cjs;
pub mod graph;
pub mod launcher;
pub mod napi_host;
pub mod runner;
pub mod transform;
