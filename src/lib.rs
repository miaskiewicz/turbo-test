//! turbo-test — native test runner internals (below the user-observable line).
//!
//! Shared substrate used by the milestone spikes and (eventually) the consolidated
//! ModuleRunner. Public surface is deliberately small: plumbing, not framework.

pub mod coverage;
pub mod graph;
pub mod napi_host;
pub mod runner;
pub mod transform;
