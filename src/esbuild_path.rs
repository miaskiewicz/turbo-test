//! The platform-specific npm esbuild entry point, shared by discovery and execution.
use std::path::{Path, PathBuf};

#[cfg(windows)]
const NAMES: &[&str] = &["esbuild.cmd", "esbuild.exe", "esbuild"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["esbuild"];

pub fn local(root: &Path) -> Option<PathBuf> {
    let bin = root.join("node_modules/.bin");
    NAMES
        .iter()
        .map(|name| bin.join(name))
        .find(|path| path.exists())
}

#[cfg(test)]
#[path = "esbuild_path_tests.rs"]
mod tests;
