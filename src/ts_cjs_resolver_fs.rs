//! A read-only config view correcting package `tsconfig` entries for Oxc's extends loader.
use super::{config, parent_path, parent_specs};
use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct ConfigFileSystem {
    os: FileSystemOs,
    configs: HashMap<PathBuf, String>,
}
impl ConfigFileSystem {
    pub(crate) fn for_config(path: Option<&Path>) -> Self {
        let mut configs = HashMap::new();
        if let Some(path) = path {
            collect(path, &mut HashSet::new(), &mut configs);
        }
        Self {
            os: FileSystemOs::new(),
            configs,
        }
    }
}
fn absolute_parent(path: &Path, value: &mut Value) {
    let Some(spec) = value.as_str() else {
        return;
    };
    if spec.starts_with('.') || Path::new(spec).is_absolute() {
        return;
    }
    if let Some(resolved) = parent_path(path, spec) {
        *value = Value::String(resolved.to_string_lossy().into_owned());
    }
}
fn rewrite(path: &Path, value: &mut Value) {
    match value.get_mut("extends") {
        Some(Value::Array(parents)) => parents
            .iter_mut()
            .for_each(|parent| absolute_parent(path, parent)),
        Some(parent) => absolute_parent(path, parent),
        None => {}
    }
}
fn collect(path: &Path, seen: &mut HashSet<PathBuf>, configs: &mut HashMap<PathBuf, String>) {
    let logical = path.to_path_buf();
    let Ok(path) = crate::canon::canonicalize(path) else {
        return;
    };
    if seen.len() >= 32 || !seen.insert(path.clone()) {
        return;
    }
    let Some(mut value) = config(&path) else {
        return;
    };
    for spec in parent_specs(&value) {
        if let Some(parent) = parent_path(&path, spec) {
            collect(&parent, seen, configs);
        }
    }
    let original = value.clone();
    rewrite(&path, &mut value);
    if value != original {
        let rewritten = value.to_string();
        configs.insert(logical, rewritten.clone());
        configs.insert(path, rewritten);
    }
}
impl FileSystem for ConfigFileSystem {
    fn new() -> Self {
        Self::for_config(None)
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.os.read(path)
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        if self.configs.is_empty() {
            return self.os.read_to_string(path);
        }
        if let Some(config) = self.configs.get(path) {
            return Ok(config.clone());
        }
        self.os.read_to_string(path)
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.metadata(path)
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.os.symlink_metadata(path)
    }
    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        self.os.read_link(path)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.os.canonicalize(path)
    }
}
