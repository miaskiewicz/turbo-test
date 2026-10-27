//! Shared helpers for the integration tests that drive the real `turbo-test` binary (and the
//! milestone spike binaries) as subprocesses against throwaway fixture projects.
//!
//! Every project lives OUTSIDE the repository (under the system temp dir) so the runner's
//! upward searches (vitest config, tsconfig, package.json `type`, `node_modules/.bin/esbuild`)
//! only see what the test put there. Each run gets its own `TMPDIR` and `TURBO_CACHE_DIR`
//! inside the project, so the duration history and transform/code caches never leak between
//! tests (or into the developer's real cache).
//!
//! Under `cargo tarpaulin` the spawned binaries are instrumented too and write their own
//! profiles; tarpaulin.toml's `objects` maps them back to source.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The repository root (where `fixtures/`, `node_modules/` and `cli.js` live).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A fresh, empty fixture project directory (removed and recreated if it already exists).
pub struct Project {
    pub dir: PathBuf,
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

impl Project {
    /// Create `<tmp>/turbo-test-it/<name>-<pid>-<n>`. `name` should identify the test.
    pub fn new(name: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join("turbo-test-it");
        std::fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let dir = base.join(format!("{name}-{}-{n}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        Project { dir }
    }

    /// Absolute path of `rel` inside the project.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    /// Write `contents` to `rel` (parent dirs created). Returns the absolute path.
    pub fn file(&self, rel: &str, contents: &str) -> PathBuf {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, contents).unwrap();
        p
    }

    /// Make the repo's installed esbuild visible as this project's `node_modules/.bin/esbuild`
    /// (the runner's project-root marker and its dep bundler). Requires `npm ci` in the repo.
    pub fn with_esbuild(&self) -> &Self {
        self.link_pkg("esbuild");
        self.link_pkg("@esbuild");
        self.link_bin("esbuild");
        self
    }

    /// Make the repo's installed TypeScript visible (`node_modules/typescript`, `.bin/tsc`).
    pub fn with_typescript(&self) -> &Self {
        self.link_pkg("typescript");
        self.link_bin("tsc");
        self
    }

    fn link_pkg(&self, name: &str) {
        let src = repo_root().join("node_modules").join(name);
        assert!(
            src.exists(),
            "{} missing — run `npm ci` in the repo first",
            src.display()
        );
        let dst = self.dir.join("node_modules").join(name);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        if !dst.exists() {
            symlink(&src, &dst);
        }
    }

    fn link_bin(&self, name: &str) {
        let src = repo_root().join("node_modules/.bin").join(name);
        assert!(
            src.exists(),
            "{} missing — run `npm ci` in the repo first",
            src.display()
        );
        let dst = self.dir.join("node_modules/.bin").join(name);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        if !dst.exists() {
            symlink(&src.canonicalize().unwrap(), &dst);
        }
    }
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) {
    std::os::unix::fs::symlink(src, dst).unwrap();
}

#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) {
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst).unwrap();
    } else {
        std::os::windows::fs::symlink_file(src, dst).unwrap();
    }
}

/// Outcome of one subprocess run.
#[derive(Debug)]
pub struct Run {
    /// Exit code (`None` when killed by a signal).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// stdout + stderr, for assertion messages.
    pub fn all(&self) -> String {
        format!(
            "--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout, self.stderr
        )
    }
}

/// Run an executable with `args` in `cwd`, an isolated TMPDIR/TURBO_CACHE_DIR under `cwd`,
/// every inherited `TURBO_*` variable, every CI variable (`turbo_test::ci::env_vars`) and every
/// AI-agent variable (`turbo_test::agent::AGENT_VARS`, `EDITOR`, `TERM_PROGRAM`) and colour
/// variable (`NO_COLOR`, `FORCE_COLOR`, `FORCE_TTY`, `TERM`) removed,
/// `TURBO_JOBS=2`, `TURBO_LEGACY_REPORTER=1` and `TURBO_CHAI` (the repo's `node_modules/chai`)
/// unless `env` overrides them, and `env` applied last.
pub fn run_exe(exe: &str, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    run_exe_unset(exe, cwd, args, env, &[])
}

/// `run_exe` with the variables `unset` removed after the others are set (`TURBO_CHAI` too).
pub fn run_exe_unset(
    exe: &str,
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    unset: &[&str],
) -> Run {
    let tmp = cwd.join(".tt-tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let mut cmd = Command::new(exe);
    cmd.current_dir(cwd).args(args);
    for (k, _) in std::env::vars() {
        if k.starts_with("TURBO_") {
            cmd.env_remove(k);
        }
    }
    // CI variables switch snapshots to vitest's CI mode (a missing snapshot fails): every
    // variable std-env's isCI reads is removed; the tests set them themselves where they check
    // that.
    for k in turbo_test::ci::env_vars() {
        cmd.env_remove(k);
    }
    // an AI agent running the tests (std-env isAgent) would switch the default reporter to
    // vitest's agent reporter: every variable it reads is removed too
    for k in turbo_test::agent::AGENT_VARS
        .iter()
        .chain(&["EDITOR", "TERM_PROGRAM"])
    {
        cmd.env_remove(k);
    }
    // vitest's colours (tinyrainbow) follow these: the tests set them where they check colours
    for k in ["NO_COLOR", "FORCE_COLOR", "FORCE_TTY", "TERM"] {
        cmd.env_remove(k);
    }
    // the tests written against turbo-test's own PASS / FAIL lines keep them; the vitest human
    // reporters (turbo-test#94) are checked with TURBO_LEGACY_REPORTER=0
    // the chai cli.js passes (turbo-test#160): the repo's own, installed by `npm ci`
    cmd.env(
        "TURBO_CHAI",
        Path::new(env!("CARGO_MANIFEST_DIR")).join("node_modules/chai/index.js"),
    );
    cmd.env("TMPDIR", &tmp)
        .env("TURBO_CACHE_DIR", cwd.join(".tt-cache"))
        .env("TURBO_JOBS", "2")
        .env("TURBO_LEGACY_REPORTER", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    for k in unset {
        cmd.env_remove(k);
    }
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {exe}: {e}"));
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run the `turbo-test` CLI (see `run_exe`).
pub fn turbo_test(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    run_exe(env!("CARGO_BIN_EXE_turbo-test"), cwd, args, env)
}

/// A `--reporter json` file entry's outcome: vitest's `passed` / `failed`, or `error` for a file
/// that failed to load (failed with no test and a load-error `message`).
pub fn file_status(t: &serde_json::Value) -> String {
    let tests = t["assertionResults"].as_array().map_or(0, Vec::len);
    if t["status"] == "failed" && tests == 0 && t["message"].as_str().is_some_and(|m| !m.is_empty())
    {
        return "error".to_string();
    }
    t["status"].as_str().unwrap().to_string()
}

/// A `--reporter json` file entry's (passed, failed) test counts.
pub fn file_counts(t: &serde_json::Value) -> (u64, u64) {
    let count = |st: &str| {
        t["assertionResults"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["status"] == st)
            .count() as u64
    };
    (count("passed"), count("failed"))
}

/// Parse the `--reporter json` artifact from stdout.
pub fn json_report(run: &Run) -> serde_json::Value {
    serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|e| panic!("not a JSON report ({e}):\n{}", run.all()))
}
