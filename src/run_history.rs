//! Per-test dependency fingerprints, captured while a test file executes.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::cli::FileResult;

pub type Fingerprints = HashMap<String, String>;
pub type History = HashMap<String, Value>;
thread_local! {
    static ACTIVE: RefCell<Option<Fingerprints>> = const { RefCell::new(None) };
}

fn cache_path(root: &Path) -> PathBuf {
    root.join("node_modules/.cache/turbo-test/last-run.json")
}

fn key(path: &Path) -> String {
    crate::canon::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn fingerprint(path: &Path) -> Option<String> {
    let contents = std::fs::read(path).ok()?;
    let modified = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{modified}:{:x}", Sha256::digest(contents)))
}

pub fn load(root: &Path) -> History {
    std::fs::read(cache_path(root))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn changed(history: &History, path: &Path) -> bool {
    let Some(record) = history.get(&key(path)) else {
        return true;
    };
    if record.get("failed").and_then(Value::as_bool) != Some(false) {
        return true;
    }
    let Some(deps) = record.get("dependencies").and_then(Value::as_object) else {
        return true;
    };
    deps.is_empty()
        || deps
            .iter()
            .any(|(file, hash)| fingerprint(Path::new(file)).as_deref() != hash.as_str())
}

pub fn begin(file: &Path) {
    if std::env::var_os("TURBO_RECORD_DEPENDENCIES").is_none() {
        return;
    }
    ACTIVE.with(|active| *active.borrow_mut() = Some(HashMap::new()));
    capture(file);
}

pub fn enabled() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

pub fn capture(path: &Path) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(deps) = active.as_mut() else { return };
        let path = key(path);
        if let std::collections::hash_map::Entry::Vacant(entry) = deps.entry(path) {
            if let Some(hash) = fingerprint(Path::new(entry.key())) {
                entry.insert(hash);
            }
        }
    });
}

pub fn capture_project_inputs(root: &Path) {
    if !enabled() {
        return;
    }
    for dir in root.ancestors() {
        for name in crate::launcher::VITEST_CONFIG_FILES.iter().chain(
            [
                "jest.config.js",
                "jest.config.cjs",
                "jest.config.mjs",
                "jest.config.ts",
                "jest.config.json",
                "package.json",
                "tsconfig.json",
            ]
            .iter(),
        ) {
            capture_config(&dir.join(name), name);
        }
    }
    for name in [".env", ".env.local", ".env.test", ".env.test.local"] {
        capture(&root.join(name));
    }
    if let Some(setup) = crate::runner::jest_global_scripts(root).and_then(|scripts| scripts.setup)
    {
        capture_with_imports(&setup);
    }
}

fn capture_config(path: &Path, name: &str) {
    capture(path);
    if path.is_file()
        && ["vitest.config.", "vite.config.", "jest.config."]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    {
        capture_with_imports(path);
    }
}

fn capture_with_imports(path: &Path) {
    capture(path);
    for dep in crate::graph::transitive_imports(path, &mut HashMap::new()) {
        capture(&dep);
    }
}

pub fn finish() -> Option<Fingerprints> {
    ACTIVE.with(|active| active.borrow_mut().take())
}

pub fn record(root: &Path, files: &[PathBuf], results: &[FileResult], partial: bool) {
    if !results.iter().any(|result| result.dependencies.is_some()) {
        return;
    }
    let mut history = load(root);
    for result in results {
        update_record(&mut history, files, result, partial);
    }
    write_history(root, &history);
}

fn update_record(history: &mut History, files: &[PathBuf], result: &FileResult, partial: bool) {
    let (Some(file), Some(deps)) = (files.get(result.idx), &result.dependencies) else {
        return;
    };
    let key = key(file);
    let only = std::fs::read_to_string(file)
        .ok()
        .is_some_and(|source| source.contains(".only(") || source.contains(".only."));
    if partial || only {
        if let Some(previous) = history.get_mut(&key) {
            merge_partial(previous, deps, result.failed_file);
        }
    } else {
        history.insert(
            key,
            json!({"failed": result.failed_file, "dependencies": deps}),
        );
    }
}

fn merge_partial(previous: &mut Value, deps: &Fingerprints, failed: bool) {
    if let Some(old) = previous
        .get_mut("dependencies")
        .and_then(Value::as_object_mut)
    {
        for (path, hash) in deps {
            old.entry(path).or_insert_with(|| json!(hash));
        }
    }
    if failed {
        previous["failed"] = json!(true);
    }
}

fn write_history(root: &Path, history: &History) {
    let path = cache_path(root);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(data) = serde_json::to_vec(&history) {
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        if std::fs::write(&temporary, data).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_failed_unrecorded_and_changed_dependencies() {
        let root = std::env::temp_dir().join(format!("turbo-test-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("a.test.js");
        let dep = root.join("dep.js");
        std::fs::write(&file, "test").unwrap();
        std::fs::write(&dep, "original").unwrap();
        let mut history = History::new();
        assert!(changed(&history, &file));
        let deps = json!({key(&file): fingerprint(&file), key(&dep): fingerprint(&dep)});
        history.insert(key(&file), json!({"failed": false, "dependencies": deps}));
        assert!(!changed(&history, &file));
        history.get_mut(&key(&file)).unwrap()["failed"] = json!(true);
        assert!(changed(&history, &file));
        history.get_mut(&key(&file)).unwrap()["failed"] = json!(false);
        std::fs::remove_file(&dep).unwrap();
        assert!(changed(&history, &file));
        let _ = std::fs::remove_dir_all(root);
    }
}
