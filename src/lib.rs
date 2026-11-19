//! turbo-test — native test runner internals (below the user-observable line).
//!
//! Shared substrate used by the milestone spikes and (eventually) the consolidated
//! ModuleRunner. Public surface is deliberately small: plumbing, not framework.

pub mod aes_compat;
pub mod agent;
pub mod async_context;
pub mod async_hooks;
pub mod async_process;
pub mod blocklist;
pub mod bundler;
pub mod canon;
pub mod ci;
pub mod cli;
pub mod colors;
pub mod coverage;
pub mod coverage_branch;
pub mod crypto_compat;
pub mod encrypted_pkcs8;
pub mod esbuild_path;
pub mod esm_cjs;
pub mod esm_spy;
pub mod failure;
#[cfg(unix)]
pub mod fs_handles;
pub mod graph;
pub mod heap_guard;
pub mod highlight;
pub mod human;
pub mod idna;
pub mod import_glob;
pub mod key_compat;
pub mod launcher;
pub mod location;
pub mod mock_read;
pub mod mock_scope;
pub mod napi_host;
pub(crate) mod native_cancellation;
pub mod node_compat;
pub mod parse_error;
pub mod pnpm_ancestor;
pub mod rsa_compat;
pub mod runner;
pub(crate) mod runtime_sources;
pub mod structured_clone;
pub mod text_decoder;
pub mod transform;
pub mod vite_asset;
pub mod vm_compat;
pub mod zlib_stream;

pub mod message_port;

pub mod process_compat;
pub mod process_usage;

pub mod http_compat;
pub mod vue;
pub mod x509_compat;

pub mod tcp_compat;

pub(crate) mod ts_cjs_interop;

pub(crate) mod runtime_profile;
