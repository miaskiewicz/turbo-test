//! Opt-in Vue SFC loading through the project's real Vue 3 compiler, before JS lowering.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COMPILER: &str = include_str!("../scripts/vue-transform.cjs");
thread_local! {
    static RUNTIME_FLAG: Cell<Option<Option<bool>>> = const { Cell::new(None) };
    static PROJECT_ENABLED: Cell<bool> = const { Cell::new(false) };
    static COMPILED: RefCell<HashMap<(PathBuf, u64), String>> = RefCell::new(HashMap::new());
}

pub fn is_component(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "vue")
}

fn manifest(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .map(|directory| directory.join("package.json"))
        .find(|path| path.is_file())
}

fn declares_plugin(from: &Path) -> bool {
    let Some(path) = manifest(from) else {
        return false;
    };
    #[cfg(test)]
    MANIFEST_READS.with(|reads| reads.set(reads.get() + 1));
    let Some(package) = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return false;
    };
    ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .any(|key| package[*key]["@vitejs/plugin-vue"].is_string())
}

pub fn prepare_project(entry: &Path) {
    PROJECT_ENABLED.with(|enabled| enabled.set(declares_plugin(entry)));
}

pub(crate) struct RuntimeFlagGuard {
    previous: Option<Option<bool>>,
}

impl Drop for RuntimeFlagGuard {
    fn drop(&mut self) {
        RUNTIME_FLAG.with(|flag| flag.set(self.previous));
    }
}

pub(crate) fn runtime_flag(value: Option<&str>) -> RuntimeFlagGuard {
    let mode = match value {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    };
    RuntimeFlagGuard {
        previous: RUNTIME_FLAG.with(|flag| flag.replace(Some(mode))),
    }
}

fn effective_flag() -> Option<bool> {
    RUNTIME_FLAG.with(|flag| flag.get()).unwrap_or_else(|| {
        match std::env::var("TURBO_VUE").ok().as_deref() {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        }
    })
}

pub fn enabled(from: &Path) -> bool {
    effective_flag()
        .unwrap_or_else(|| PROJECT_ENABLED.with(|enabled| enabled.get()) || declares_plugin(from))
}

pub fn runtime_esm(specifier: &str, from: &Path) -> bool {
    matches!(
        specifier,
        "vue" | "@vue/shared" | "@vue/reactivity" | "@vue/runtime-core" | "@vue/runtime-dom"
    ) && enabled(from)
}

/// Choose only a genuine Vue package's bundled runtime, after project mappings resolve.
pub fn runtime_path(specifier: &str, resolved: &Path) -> Option<PathBuf> {
    let package = runtime_package(specifier, resolved)?;
    let name = if specifier == "vue" {
        "vue"
    } else {
        specifier.strip_prefix("@vue/")?
    };
    let path = package
        .parent()?
        .join(format!("dist/{name}.esm-bundler.js"));
    crate::canon::canonicalize(path)
        .ok()
        .filter(|path| path.is_file())
}

fn runtime_package(specifier: &str, resolved: &Path) -> Option<PathBuf> {
    let package = manifest(resolved.parent()?)?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&package).ok()?).ok()?;
    (metadata["name"].as_str()? == specifier).then_some(package)
}

fn stamp(path: &Path, source: &str) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hash);
    COMPILER.hash(&mut hash);
    for directory in path.ancestors() {
        for name in [
            "node_modules/vue/package.json",
            "node_modules/@vue/compiler-sfc/package.json",
        ] {
            std::fs::read(directory.join(name)).ok().hash(&mut hash);
        }
    }
    hash.finish()
}

fn request(child: &mut std::process::Child, input: &[u8]) -> Result<(), String> {
    let Some(mut stdin) = child.stdin.take() else {
        return Err("compiler stdin unavailable".into());
    };
    stdin.write_all(input).map_err(|error| error.to_string())
}

fn invoke(path: &Path, source: &str) -> Result<String, String> {
    let node = std::env::var_os("TURBO_NODE_EXECUTABLE").unwrap_or_else(|| "node".into());
    let filename = path.to_str().ok_or("Vue SFC paths must be valid UTF-8")?;
    let input = serde_json::json!({ "filename": filename, "source": source }).to_string();
    let mut child = Command::new(node)
        .args(["--eval", COMPILER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start Node Vue compiler: {error}"))?;
    if let Err(error) = request(&mut child, input.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    result["code"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Vue compiler returned no code".into())
}

fn compiled(path: &Path, source: &str) -> Result<String, String> {
    let key = (path.to_path_buf(), stamp(path, source));
    if let Some(code) = COMPILED.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(code);
    }
    let code = invoke(path, source)?;
    COMPILED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, code.clone());
    });
    Ok(code)
}

fn javascript(path: &Path) -> Result<String, String> {
    let source = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let generated = compiled(path, &source)?;
    // Vue's compiler preserves TS syntax: use the existing real oxc parser/transformer.
    crate::transform::transform(&path.with_extension("vue.ts"), &generated)
}

/// A real SFC result for either loader graph; errors stay explicit rather than parsing HTML as JS.
pub fn load(path: &Path, as_cjs: bool) -> Result<String, String> {
    if !enabled(path) {
        return Err(format!(
            "Vue SFC support requires TURBO_VUE=1 or a declared @vitejs/plugin-vue: {}",
            path.display()
        ));
    }
    if crate::coverage::enabled() {
        return Err("Vue SFC coverage source mapping is not supported yet".into());
    }
    let code = javascript(path)
        .map_err(|error| format!("Vue SFC compile failed: {}: {error}", path.display()))?;
    if !as_cjs {
        return Ok(code);
    }
    crate::esm_cjs::emit_with(&path.with_extension("vue.js"), &code, false)
        .ok_or_else(|| format!("Vue SFC JavaScript lowering failed: {}", path.display()))
}

#[cfg(test)]
thread_local! { static MANIFEST_READS: Cell<usize> = const { Cell::new(0) }; }
#[cfg(test)]
#[path = "vue_plugin_memo_tests.rs"]
mod plugin_memo_tests;
