//! Optional Vite graph collection for plugin-owned watch files.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

type Dependencies = HashMap<String, Vec<PathBuf>>;
const SCRIPT: &str = include_str!("vite_watch.cjs");
const MARKER: &str = "__TURBO_VITE_WATCH__";

/// Find a config that may contain plugins. Configs without a statically identifiable plugin
/// declaration and without imports can be handled entirely by the native resolver.
pub fn config(root: &Path, forced: Option<&Path>) -> Option<PathBuf> {
    let config = forced
        .map(Path::to_path_buf)
        .or_else(|| find_config(root))?;
    let Ok(source) = std::fs::read_to_string(&config) else {
        return Some(config);
    };
    let source = crate::launcher::strip_comments(&source);
    [
        "plugins",
        "mergeConfig",
        "extends",
        "import",
        "require",
        "(",
    ]
    .iter()
    .any(|needle| source.contains(needle))
    .then_some(config)
}

fn find_config(root: &Path) -> Option<PathBuf> {
    root.ancestors().find_map(|dir| {
        crate::launcher::VITEST_CONFIG_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file())
    })
}

/// Resolve graph edges before a run. Failure must be treated as an unknown graph, never empty.
pub fn collect(root: &Path, config: &Path, tests: &[PathBuf]) -> Result<Dependencies, String> {
    collect_with_loaded(root, config, tests, &HashMap::new())
}

pub fn collect_with_loaded(
    root: &Path,
    config: &Path,
    tests: &[PathBuf],
    loaded: &HashMap<String, Vec<PathBuf>>,
) -> Result<Dependencies, String> {
    let entries: Vec<_> = tests
        .iter()
        .map(|test| {
            let file = absolute(test);
            let mut deps = crate::runner::dependency_setup_files(&file);
            deps.push(file.clone());
            if let Some(extra) = loaded.get(&file.to_string_lossy().into_owned()) {
                deps.extend(extra.iter().cloned());
            }
            json!({ "file": file, "entries": deps })
        })
        .collect();
    let input = json!({ "root": root, "config": config, "tests": entries }).to_string();
    let mut child = Command::new("node")
        .args(["-e", SCRIPT])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start Vite helper: {e}"))?;
    if let Err(e) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot send inputs to Vite helper: {e}"));
    }
    // stdout is drained concurrently: plugin logs and large graphs can fill a pipe.
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        let mut stdout = stdout;
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = reader.join().unwrap_or_default();
                if !status.success() {
                    return Err("Vite plugin graph collection failed".into());
                }
                return validate_graph(parse(&output)?, tests);
            }
            Ok(None) if started.elapsed() < Duration::from_secs(60) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("Vite plugin graph collection timed out or failed".into());
            }
        }
    }
}

fn parse(output: &str) -> Result<Dependencies, String> {
    let line = output
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(MARKER))
        .ok_or_else(|| "Vite helper returned no dependency graph".to_string())?;
    serde_json::from_str(line).map_err(|e| format!("invalid Vite dependency graph: {e}"))
}

fn validate_graph(graph: Dependencies, tests: &[PathBuf]) -> Result<Dependencies, String> {
    if tests
        .iter()
        .any(|test| !graph.contains_key(absolute(test).to_string_lossy().as_ref()))
    {
        return Err("Vite helper returned an incomplete dependency graph".into());
    }
    Ok(graph)
}

pub fn absolute(path: &Path) -> PathBuf {
    crate::canon::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_framed_graph_and_rejects_incomplete_output() {
        let output = "plugin log\n__TURBO_VITE_WATCH__{\"/tmp/a.test.js\":[\"/tmp/theme.json\"]}\n";
        let graph = parse(output).unwrap();
        assert_eq!(
            graph["/tmp/a.test.js"],
            vec![PathBuf::from("/tmp/theme.json")]
        );
        assert!(parse("not a graph").is_err());
        assert!(parse("__TURBO_VITE_WATCH__{").is_err());
        assert!(validate_graph(graph.clone(), &[PathBuf::from("/tmp/a.test.js")]).is_ok());
        assert!(validate_graph(graph, &[PathBuf::from("/tmp/b.test.js")]).is_err());
    }

    #[test]
    fn recognizes_dynamic_config_and_missing_forced_config() {
        let root =
            std::env::temp_dir().join(format!("turbo-vite-watch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(config(&root, None).is_none());
        let file = root.join("vite.config.js");
        std::fs::write(&file, "export default { test: { environment: 'node' } }").unwrap();
        assert!(config(&root, None).is_none());
        std::fs::write(&file, "export default { plugins: [] }").unwrap();
        assert_eq!(config(&root, None), Some(file));
        let missing = root.join("missing.js");
        assert_eq!(config(&root, Some(&missing)), Some(missing));
        let _ = std::fs::remove_dir_all(root);
    }
}
