//! turbo-test — native test runner internals (below the user-observable line).
//!
//! Shared substrate used by the milestone spikes and (eventually) the consolidated
//! ModuleRunner. Public surface is deliberately small: plumbing, not framework.

pub mod agent;
pub mod bundler;
pub mod canon;
pub mod ci;
pub mod cli;
pub mod colors;
pub mod coverage;
pub mod coverage_branch;
pub mod crypto_compat;
pub mod esbuild_path;
pub mod esm_cjs;
pub mod esm_spy;
pub mod failure;
pub mod graph;
pub mod heap_guard;
pub mod highlight;
pub mod human;
pub mod idna;
pub mod import_glob;
pub mod launcher;
pub mod location;
pub mod mock_read;
pub mod mock_scope;
pub mod napi_host;
pub mod node_compat;
pub mod parse_error;
pub mod runner;
pub mod structured_clone;
pub mod text_decoder;
pub mod transform;
pub mod vite_asset;
