//! CLI launcher — the front-end work that used to live in `cli.js` (the npm Node launcher),
//! ported to Rust so the binary is self-contained (no Node process needed to launch a run).
//!
//! Responsibilities (all formerly in cli.js):
//!   - strip a leading `run`/`watch`/`dev` subcommand (vitest dispatch token);
//!   - consume launcher-only flags (`-c/--config`, `--root/--dir`, `--environment`,
//!     `--changed [since]`, `--isolate/--no-isolate`, `--globals/--no-globals`,
//!     `--passWithNoTests`) and turn them into env vars / discovery options;
//!   - default test-file discovery (vitest-style) honoring a vitest/vite config's
//!     test-level `include`/`exclude` globs;
//!   - inject the vitest `coverage` block's thresholds/include/exclude when `--coverage*` is on
//!     and the user didn't pass them explicitly (flags win);
//!   - `--changed [since]` git filter;
//!   - prune file args that no longer exist.
//!
//! `prepare()` returns the effective argument vector — the runner-side flags it did NOT consume,
//! followed by the resolved absolute test-file paths — which `turbo_test.rs`'s existing flag loop
//! then parses unchanged. It may `std::process::exit` directly for the terminal cases
//! (`--passWithNoTests`, no files found, `--changed` with nothing changed), exactly as cli.js did.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::coverage::glob_matches;

/// Directories never descended into during default discovery (mirrors cli.js `SKIP_DIR`).
const SKIP_DIR: [&str; 8] =
    ["node_modules", ".git", "dist", "build", "coverage", ".next", ".turbo", "target"];

/// vitest/vite config filenames, in cli.js precedence order.
const CONFIG_NAMES: [&str; 8] = [
    "vitest.config.ts",
    "vitest.config.mts",
    "vitest.config.js",
    "vitest.config.mjs",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.js",
    "vite.config.mjs",
];

/// Runner flags that take a following value (so a file arg isn't mistaken for the value, and the
/// value isn't mistaken for a file). Mirrors the value-flag set cli.js forwarded with their arg.
const VALUE_FLAGS: [&str; 21] = [
    "-j",
    "--jobs",
    "--shard",
    "--reporter",
    "--reporters",
    "--outputFile",
    "--output-file",
    "-t",
    "--testNamePattern",
    "--testTimeout",
    "--retry",
    "--bail",
    "--maxWorkers",
    "--minWorkers",
    "--coverage-dir",
    "--coverage-thresholds",
    "--coverage-threshold",
    "--coverage-reporter",
    "--coverage-reporters",
    "--coverage-include",
    "--coverage-exclude",
];

/// `name` is a vitest-style test file: `*.{test,spec}.{ts,tsx,js,jsx,mts,cts}` (cli.js TEST_RE).
fn is_test_file(name: &str) -> bool {
    // strip the final extension, require it to be one of the known ones, then require the
    // remaining stem to end with `.test` or `.spec`.
    let exts = ["ts", "tsx", "js", "jsx", "mts", "cts"];
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    if !exts.contains(&ext) {
        return false;
    }
    stem.ends_with(".test") || stem.ends_with(".spec")
}

/// Recursive default discovery walk (cli.js `walk`): collect test files, skipping hidden entries
/// and the SKIP_DIR set.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let full = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                if !SKIP_DIR.contains(&name.as_ref()) {
                    walk(&full, out);
                }
            }
            Ok(ft) if ft.is_file() => {
                if is_test_file(&name) {
                    out.push(full);
                }
            }
            _ => {}
        }
    }
}

/// Skip ASCII whitespace forward from `i`. Returns the new index.
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Find the array literal for `key` — `key` followed by optional ws, `:`, optional ws, `[ ... ]` —
/// and return the quoted string items inside it. Mirrors cli.js `arr(key)` (a loose string-scan,
/// not a real parse): `key` is matched as a substring, the FIRST such occurrence whose punctuation
/// lines up wins. Returns `None` when no such `key:[...]` exists.
fn scan_string_array(text: &str, key: &str) -> Option<Vec<String>> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let after = from + rel + key.len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'[' {
                // read to the closing ']' ( [^\]]* — no nested brackets, like the JS regex)
                if let Some(close_rel) = text[i + 1..].find(']') {
                    let inner = &text[i + 1..i + 1 + close_rel];
                    return Some(extract_quoted(inner));
                }
            }
        }
        from = after;
    }
    None
}

/// Pull every quoted (`'` `"` or `` ` ``) string out of `s`, in order. Mirrors cli.js
/// `m[1].match(/['"`]([^'"`]+)['"`]/g)`.
fn extract_quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' || c == b'"' || c == b'`' {
            if let Some(end_rel) = s[i + 1..].find(|ch| ch == '\'' || ch == '"' || ch == '`') {
                let val = &s[i + 1..i + 1 + end_rel];
                if !val.is_empty() {
                    out.push(val.to_string());
                }
                i = i + 1 + end_rel + 1;
                continue;
            } else {
                break;
            }
        }
        i += 1;
    }
    out
}

/// A discovered vitest config and its directory (the discovery/project root).
struct Config {
    dir: PathBuf,
    text: String,
}

/// Locate a vitest/vite config: with `forced` (a `-c/--config` path) use that exact file (its dir
/// is the root); otherwise walk up from `start_dir`. Mirrors cli.js `findConfig`.
fn find_config(start_dir: &Path, forced: Option<&str>) -> Option<Config> {
    if let Some(f) = forced {
        let p = std::path::absolute(f).unwrap_or_else(|_| PathBuf::from(f));
        let text = std::fs::read_to_string(&p).ok()?;
        let dir = p.parent().unwrap_or(Path::new(".")).to_path_buf();
        return Some(Config { dir, text });
    }
    let mut dir = start_dir.to_path_buf();
    loop {
        for n in CONFIG_NAMES {
            let p = dir.join(n);
            if p.is_file() {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    return Some(Config { dir: dir.clone(), text });
                }
                return None;
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// Test-level include/exclude globs + their root, from a discovered config. `None` when there's no
/// `test.include` (→ caller falls back to default discovery). Mirrors cli.js `patternsFromText`:
/// the FIRST `include`/`exclude` arrays in the file are the test-level ones (they precede any
/// `coverage.*` block in the config object).
struct Patterns {
    root: PathBuf,
    include: Vec<String>,
    exclude: Vec<String>,
}

fn vitest_patterns(start_dir: &Path, forced: Option<&str>) -> Option<Patterns> {
    let cfg = find_config(start_dir, forced)?;
    // Scan include/exclude in the TEST config only — truncate before the coverage block. Otherwise
    // a project with no `test.exclude` but a `coverage.exclude` whose first entry is the test glob
    // (e.g. `**/*.test.{ts,tsx}`, common in vitest coverage configs) has that picked up as the test
    // exclude → every test file is excluded → "no test files found". test.* precedes coverage.* in
    // the config object, so the prefix before `coverage:` holds the real test patterns.
    let scan_text = match find_coverage_block(&cfg.text) {
        Some(ci) => &cfg.text[..ci],
        None => cfg.text.as_str(),
    };
    let include = scan_string_array(scan_text, "include")?; // no test.include → default discovery
    let exclude = scan_string_array(scan_text, "exclude").unwrap_or_default();
    Some(Patterns { root: cfg.dir, include, exclude })
}

/// Read `test.environment` (node|jsdom|happy-dom) from config text. Mirrors cli.js
/// `configEnvironment`: `environment\s*:\s*['"`]([a-z-]+)['"`]`, lowercased.
fn config_environment(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("environment") {
        let after = from + rel + "environment".len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && (b[i] == b'\'' || b[i] == b'"' || b[i] == b'`') {
                let rest = &text[i + 1..];
                let val: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
                    .collect();
                if !val.is_empty() {
                    return Some(val.to_ascii_lowercase());
                }
            }
        }
        from = after;
    }
    None
}

/// The vitest `coverage` block's include/exclude globs + thresholds string. Mirrors cli.js
/// `vitestCoverage`. Returns `None` only when there's no config at all.
struct CoverageCfg {
    include: Vec<String>,
    exclude: Vec<String>,
    thresholds: Option<String>,
}

fn vitest_coverage(start_dir: &Path, forced: Option<&str>) -> Option<CoverageCfg> {
    let cfg = find_config(start_dir, forced)?;
    let text = &cfg.text;
    // slice from the `coverage:` key so include/exclude/thresholds resolve to the coverage block,
    // not the test-level ones (test.* precedes coverage.* in the config object).
    let Some(ci) = find_coverage_block(text) else {
        return Some(CoverageCfg { include: vec![], exclude: vec![], thresholds: None });
    };
    let slice = &text[ci..];
    let include = scan_string_array(slice, "include").unwrap_or_default();
    let exclude = scan_string_array(slice, "exclude").unwrap_or_default();
    // thresholds can be `coverage.thresholds: { lines: 90, ... }` or the flat legacy form
    // (numbers directly under coverage). Prefer the nested block when present.
    let thr_text = scan_braced(slice, "thresholds").unwrap_or_else(|| slice.to_string());
    let mut parts = Vec::new();
    for k in ["lines", "functions", "branches", "statements"] {
        if let Some(v) = scan_number(&thr_text, k) {
            parts.push(format!("{k}={v}"));
        }
    }
    let thresholds = if parts.is_empty() { None } else { Some(parts.join(",")) };
    Some(CoverageCfg { include, exclude, thresholds })
}

/// Byte offset of the `coverage` key whose value is an object (`coverage\s*:\s*\{`).
fn find_coverage_block(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("coverage") {
        let at = from + rel;
        let after = at + "coverage".len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'{' {
                return Some(at);
            }
        }
        from = after;
    }
    None
}

/// Contents between the braces of `key\s*:\s*\{ ... \}` (first match, no nesting — like the JS
/// `thresholds\s*:\s*\{([^}]*)\}`).
fn scan_braced(text: &str, key: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let after = from + rel + key.len();
        let i = skip_ws(b, after);
        if i < b.len() && b[i] == b':' {
            let i = skip_ws(b, i + 1);
            if i < b.len() && b[i] == b'{' {
                if let Some(close_rel) = text[i + 1..].find('}') {
                    return Some(text[i + 1..i + 1 + close_rel].to_string());
                }
            }
        }
        from = after;
    }
    None
}

/// Numeric value for `key` (`(?:^|[^.\w])key\s*:\s*(\d+(?:\.\d+)?)`) — the leading guard prevents
/// matching `key` as the tail of another identifier (so `branches` doesn't match `subBranches`).
fn scan_number(text: &str, key: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(key) {
        let at = from + rel;
        // guard: preceding char must not be '.', '_', alphanumeric (i.e. not part of a longer word)
        let ok_prev = at == 0
            || {
                let p = b[at - 1];
                p != b'.' && p != b'_' && !p.is_ascii_alphanumeric()
            };
        let after = at + key.len();
        if ok_prev {
            let i = skip_ws(b, after);
            if i < b.len() && b[i] == b':' {
                let mut i = skip_ws(b, i + 1);
                let start = i;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                if i > start {
                    return Some(text[start..i].to_string());
                }
            }
        }
        from = after;
    }
    None
}

/// Default discovery: all test files under `cwd`, filtered by config include/exclude when a config
/// with `test.include` is found. Mirrors cli.js `discover`.
fn discover(cwd: &Path, forced_config: Option<&str>) -> Vec<PathBuf> {
    let mut all = Vec::new();
    walk(cwd, &mut all);
    let Some(pats) = vitest_patterns(cwd, forced_config) else {
        all.sort();
        return all;
    };
    let rel = |f: &Path| -> String {
        // project-root-relative POSIX path (vitest matches globs against this).
        let r = f.strip_prefix(&pats.root).unwrap_or(f);
        r.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    };
    let mut kept: Vec<PathBuf> = all
        .into_iter()
        .filter(|f| {
            let r = rel(f);
            pats.include.iter().any(|g| glob_matches(g, &r))
                && !pats.exclude.iter().any(|g| glob_matches(g, &r))
        })
        .collect();
    kept.sort();
    kept
}

/// `--changed [since]`: absolute paths git reports changed vs `since` (default working-tree vs
/// HEAD/index) — staged + unstaged + untracked. `None` when git is unavailable / not a repo
/// (caller then runs everything). Mirrors cli.js `gitChanged`.
fn git_changed(since: &str, cwd: &Path) -> Option<HashSet<PathBuf>> {
    let run = |args: &[&str]| -> Option<Vec<String>> {
        let out = Command::new("git").args(args).current_dir(cwd).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(|s| s.to_string())
                .collect(),
        )
    };
    let root_lines = run(&["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(root_lines.first()?);
    let mut set = HashSet::new();
    let diff_ref: Vec<&str> = if !since.is_empty() && since != "true" { vec![since] } else { vec![] };
    let mut cmds: Vec<Vec<&str>> = vec![
        {
            let mut v = vec!["diff", "--name-only"];
            v.extend(&diff_ref);
            v
        },
        {
            let mut v = vec!["diff", "--name-only", "--cached"];
            v.extend(&diff_ref);
            v
        },
        vec!["ls-files", "--others", "--exclude-standard"],
    ];
    for args in cmds.drain(..) {
        if let Some(lines) = run(&args) {
            for f in lines {
                set.insert(repo_root.join(f));
            }
        }
    }
    Some(set)
}

/// Launcher-consumed options.
struct Opts {
    config: Option<String>,
    root: Option<String>,
    environment: Option<String>,
    /// `Some("true")` for bare `--changed`, `Some(<ref>)` for `--changed <ref>`, `None` if absent.
    changed: Option<String>,
    isolate: Option<bool>,
    pass_with_no_tests: bool,
}

/// Whether `a` is a `--key` / `-k` whose VALUE is the following token (so we don't split a file off
/// as its value when forwarding).
fn flag_takes_value(a: &str) -> bool {
    VALUE_FLAGS.contains(&a)
}

/// Whether the forwarded-flag list already carries `needle` (bare or `needle=…` inline form) — so
/// a user-passed coverage flag wins over the config-injected default.
fn has_flag(forward: &[String], needle: &str) -> bool {
    forward.iter().any(|f| f == needle || f.starts_with(&format!("{needle}=")))
}

/// Port of cli.js `main()` up to the point of spawning the native binary. Returns the effective
/// argv (forwarded runner flags + resolved absolute test files) for `turbo_test.rs` to parse, or
/// exits the process for the terminal cases. `raw` is argv WITHOUT the program name.
pub fn prepare(mut raw: Vec<String>) -> Vec<String> {
    // Accept-and-strip a leading vitest subcommand (`run`/`watch`/`dev`) — turbo-test is always a
    // single run; letting it reach the runner would look like a phantom test-file path.
    if raw.first().map(|s| matches!(s.as_str(), "run" | "watch" | "dev")).unwrap_or(false) {
        raw.remove(0);
    }

    let mut opts = Opts {
        config: None,
        root: None,
        environment: None,
        changed: None,
        isolate: None,
        pass_with_no_tests: false,
    };
    let mut forward: Vec<String> = Vec::new(); // runner-side flags, forwarded verbatim
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < raw.len() {
        let a = raw[i].clone();
        // `--key=value` inline form for launcher value-flags.
        let (key, inline_val): (&str, Option<String>) =
            if a.starts_with("--") {
                if let Some(eq) = a.find('=') {
                    (&a[..eq], Some(a[eq + 1..].to_string()))
                } else {
                    (a.as_str(), None)
                }
            } else {
                (a.as_str(), None)
            };
        // take the value for a launcher value-flag: inline `=v`, else the next non-flag token.
        let mut take_val = || -> Option<String> {
            if let Some(v) = inline_val.clone() {
                return Some(v);
            }
            if i + 1 < raw.len() && !raw[i + 1].starts_with('-') {
                i += 1;
                return Some(raw[i].clone());
            }
            None
        };

        match key {
            "--passWithNoTests" => {
                opts.pass_with_no_tests = true;
                i += 1;
                continue;
            }
            "-c" | "--config" => {
                opts.config = take_val();
                i += 1;
                continue;
            }
            // vitest distinguishes --root (project root) from --dir (test scan dir); turbo-test
            // scans the project root, so both override the discovery directory (last wins).
            "--root" | "--dir" => {
                opts.root = take_val();
                i += 1;
                continue;
            }
            "--environment" => {
                opts.environment = take_val();
                i += 1;
                continue;
            }
            // `--changed`'s arg is OPTIONAL: a following non-flag token is the `since` ref.
            "--changed" => {
                opts.changed = Some(take_val().unwrap_or_else(|| "true".to_string()));
                i += 1;
                continue;
            }
            "--isolate" => {
                opts.isolate = Some(true);
                i += 1;
                continue;
            }
            "--no-isolate" => {
                opts.isolate = Some(false);
                i += 1;
                continue;
            }
            // Globals are ALWAYS on in turbo-test; accept both spellings as no-ops.
            "--globals" | "--no-globals" => {
                i += 1;
                continue;
            }
            _ => {}
        }

        if a.starts_with('-') {
            forward.push(a.clone());
            // forward a value-flag's following value too (so it isn't seen as a file).
            if flag_takes_value(&a) && i + 1 < raw.len() && !raw[i + 1].starts_with('-') {
                forward.push(raw[i + 1].clone());
                i += 1;
            }
        } else if !a.is_empty() {
            // skip empty args (e.g. a stray "" from a shell wrapper) — they'd be treated as a
            // missing file and abort the whole run.
            files.push(a.clone());
        }
        i += 1;
    }

    // Discovery root: --root/--dir override cwd.
    let cwd: PathBuf = match &opts.root {
        Some(r) => std::path::absolute(r).unwrap_or_else(|_| PathBuf::from(r)),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // `--isolate` / `--no-isolate` → the runner's reuse env switches (read per-file in runner.rs).
    match opts.isolate {
        Some(false) => std::env::set_var("TURBO_REUSE_ISOLATE", "1"),
        Some(true) => std::env::set_var("TURBO_NO_REUSE", "1"),
        None => {}
    }

    // `--environment` → TURBO_ENV (falls back to config `test.environment`). A per-file pragma
    // overrides this at runtime (runner.rs).
    let environment = opts.environment.clone().or_else(|| {
        find_config(&cwd, opts.config.as_deref()).and_then(|c| config_environment(&c.text))
    });
    if let Some(env) = environment {
        std::env::set_var("TURBO_ENV", env);
    }

    // Coverage: fill thresholds/include/exclude from vitest config unless passed explicitly.
    let coverage_on = forward.iter().any(|f| f.starts_with("--coverage"));
    if coverage_on {
        if let Some(cov) = vitest_coverage(&cwd, opts.config.as_deref()) {
            if let Some(thr) = &cov.thresholds {
                if !has_flag(&forward, "--coverage-thresholds") && !has_flag(&forward, "--coverage-threshold") {
                    forward.push("--coverage-thresholds".to_string());
                    forward.push(thr.clone());
                }
            }
            if !cov.include.is_empty() && !has_flag(&forward, "--coverage-include") {
                forward.push("--coverage-include".to_string());
                forward.push(cov.include.join(","));
            }
            if !cov.exclude.is_empty() && !has_flag(&forward, "--coverage-exclude") {
                forward.push("--coverage-exclude".to_string());
                forward.push(cov.exclude.join(","));
            }
        }
    }

    // Per-test timeout: honor the vitest config `test.testTimeout` unless the user passed
    // `--testTimeout` explicitly. Without this the binary falls back to vitest's 5000ms default,
    // so a project that raises testTimeout (e.g. 10000 for slow editor/auto-hide flows) spuriously
    // times out under turbo-test.
    if !has_flag(&forward, "--testTimeout") {
        if let Some(cfg) = find_config(&cwd, opts.config.as_deref()) {
            if let Some(tt) = scan_number(&cfg.text, "testTimeout") {
                forward.push("--testTimeout".to_string());
                forward.push(tt);
            }
        }
    }

    // Resolve the test file set.
    let mut test_files: Vec<PathBuf> = if files.is_empty() {
        let discovered = discover(&cwd, opts.config.as_deref());
        if discovered.is_empty() {
            if opts.pass_with_no_tests {
                eprintln!("turbo-test: no test files found — exiting 0 (--passWithNoTests).");
                std::process::exit(0);
            }
            eprintln!("turbo-test: no test files found (looked for *.test.* / *.spec.*).");
            std::process::exit(1);
        }
        discovered
    } else {
        files.iter().map(PathBuf::from).collect()
    };

    // `--changed [since]`: keep only discovered test files git reports changed. Direct changed-file
    // filter (no import graph). Nothing changed → running zero tests is expected → exit 0.
    if let Some(since) = &opts.changed {
        match git_changed(since, &cwd) {
            None => {
                eprintln!("turbo-test: --changed: git unavailable / not a repo — running all discovered tests.");
            }
            Some(changed) => {
                let before = test_files.len();
                test_files.retain(|f| {
                    let abs = std::path::absolute(f).unwrap_or_else(|_| f.clone());
                    changed.contains(&abs)
                });
                if test_files.is_empty() {
                    eprintln!("turbo-test: --changed: no changed test files (of {before}) — exiting 0.");
                    std::process::exit(0);
                }
            }
        }
    }

    // Drop file args that no longer exist (stale paths would reach the runner as hard load-errors
    // and flip the exit code). Warn, don't fail.
    let missing: Vec<&PathBuf> = test_files.iter().filter(|f| !f.exists()).collect();
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|f| f.display().to_string()).collect();
        eprintln!("turbo-test: skipping {} missing file(s): {}", missing.len(), names.join(", "));
        test_files.retain(|f| f.exists());
    }
    if test_files.is_empty() {
        eprintln!("turbo-test: no existing test files to run.");
        std::process::exit(0); // nothing to run is not a failure
    }

    // Effective argv for the runner loop: forwarded flags, then resolved files.
    let mut out = forward;
    out.extend(test_files.into_iter().map(|f| f.display().to_string()));
    out
}
