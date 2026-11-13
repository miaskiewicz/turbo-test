//! Opt-in Vue SFC loading through the project's real Vue 3 compiler, before JS lowering.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COMPILER: &str = include_str!("../scripts/vue-transform.cjs");
thread_local! {
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

pub fn enabled(from: &Path) -> bool {
    match std::env::var("TURBO_VUE").ok().as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => PROJECT_ENABLED.with(|enabled| enabled.get()) || declares_plugin(from),
    }
}

/// Keep the runtime compiler available while sharing Vue's runtime between both graphs.
pub fn runtime_specifier<'a>(specifier: &'a str, from: &Path) -> &'a str {
    if specifier == "vue" && enabled(from) {
        "vue/dist/vue.esm-bundler.js"
    } else {
        specifier
    }
}

/// An opted-in Vue application has one runtime across import and require module graphs.
pub fn runtime_esm(specifier: &str, from: &Path) -> bool {
    matches!(
        specifier,
        "vue" | "@vue/shared" | "@vue/reactivity" | "@vue/runtime-core" | "@vue/runtime-dom"
    ) && enabled(from)
}

/// Only opt-in local extension/index probing; the resolver's global extension list is unchanged.
pub fn resolve(from: &Path, specifier: &str) -> Option<PathBuf> {
    if !enabled(from) || !(specifier.starts_with('.') || Path::new(specifier).is_absolute()) {
        return None;
    }
    let base = from.join(specifier);
    if base.extension().is_some() {
        return None;
    }
    [base.with_extension("vue"), base.join("index.vue")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| crate::canon::canonicalize(candidate).ok())
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
