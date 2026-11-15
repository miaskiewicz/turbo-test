//! TypeScript's explicit CommonJS import policy, including inherited JSONC options.
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Options {
    module: Option<String>,
    interop: Option<bool>,
}
impl Options {
    fn overlay(&mut self, other: Self) {
        self.module = other.module.or(self.module.take());
        self.interop = other.interop.or(self.interop);
    }
}
fn config(path: &Path) -> Option<Value> {
    let mut text = std::fs::read_to_string(path).ok()?;
    json_strip_comments::strip(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}
fn parent_path(path: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with('.') {
        let parent = path.parent()?.join(spec);
        return Some(if parent.is_file() {
            parent
        } else {
            PathBuf::from(format!("{}.json", parent.to_string_lossy()))
        });
    }
    oxc_resolver::Resolver::default()
        .resolve(path.parent()?, spec)
        .ok()
        .map(|result| result.full_path())
}
fn parent_specs(value: &Value) -> Vec<&str> {
    match &value["extends"] {
        Value::String(spec) => vec![spec.as_str()],
        Value::Array(specs) => specs.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}
fn enter_config(path: &Path, stack: &mut HashSet<PathBuf>) -> Option<PathBuf> {
    let path = crate::canon::canonicalize(path).ok()?;
    if stack.len() >= 32 || !stack.insert(path.clone()) {
        return None;
    }
    Some(path)
}
fn options(path: &Path, stack: &mut HashSet<PathBuf>) -> Option<Options> {
    let path = enter_config(path, stack)?;
    let value = config(&path)?;
    let mut result = Options::default();
    for spec in parent_specs(&value) {
        result.overlay(options(&parent_path(&path, spec)?, stack)?);
    }
    result.overlay(Options {
        module: value["compilerOptions"]["module"]
            .as_str()
            .map(str::to_owned),
        interop: value["compilerOptions"]["esModuleInterop"].as_bool(),
    });
    stack.remove(&path);
    Some(result)
}
pub(crate) fn enabled(config: &Path) -> bool {
    options(config, &mut HashSet::new()).is_some_and(|options| {
        options
            .module
            .as_deref()
            .is_some_and(|module| module.eq_ignore_ascii_case("commonjs"))
            && options.interop == Some(false)
    })
}

#[cfg(test)]
#[path = "ts_cjs_interop_tests.rs"]
mod tests;
