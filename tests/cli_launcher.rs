//! The launcher front-end through the real binary: terminal exits and their messages, positional
//! filters, subcommand stripping, `--environment` / config environment, config aliases,
//! a forced `-c` config, `--root`, and `--changed` against a real git repository (plus the
//! git-unavailable fallback).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project, Run};
use std::path::Path;
use std::process::Command;

fn tt(p: &Project, args: &[&str]) -> Run {
    turbo_test(&p.dir, args, &[])
}

const PASS: &str = "test('p', () => { expect(1).toBe(1) })\n";

/// vitest 4.1.12's "No test files found" lines with the default include / exclude (`filter`:
/// its `filter: …` line, or empty).
fn no_test_files(code: i32, filter: &str) -> String {
    format!(
        "No test files found, exiting with code {code}\n\n{filter}include: **/*.{{test,spec}}.?(c|m)[jt]s?(x)\nexclude:  **/node_modules/**, **/.git/**\n"
    )
}

#[test]
fn no_test_files_exit_codes() {
    let p = Project::new("launch-empty");
    p.file("src/util.ts", "export const x = 1\n");
    let r = tt(&p, &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    // vitest 4.1.12's message (turbo-test#59)
    assert_eq!(r.stderr, no_test_files(1, ""));
    assert_eq!(r.stdout, "");
    let r = tt(&p, &["run", "--passWithNoTests"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(r.stderr, no_test_files(0, ""));
}

#[test]
fn positional_arguments_are_filters() {
    // turbo-test#58, as vitest 4.1.12: every positional argument filters the discovered test
    // files (a root-relative path containing it, case-insensitively); one matching nothing is
    // no warning, all matching nothing is "No test files found" with exit 1
    let p = Project::new("launch-filters");
    p.file("a.test.ts", PASS);
    p.file("a/x.test.ts", PASS);
    p.file("a/b/y.test.ts", PASS);
    let r = tt(&p, &["a.test.ts", "gone.test.ts"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(r.stderr, "", "{}", r.all());
    assert!(
        r.stdout.contains("PASS  a.test.ts  (1 passed, 0 failed)"),
        "{}",
        r.all()
    );
    assert!(!r.stdout.contains("x.test.ts"), "{}", r.all());
    let r = tt(&p, &["X.TEST"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("PASS  a/x.test.ts"), "{}", r.all());
    assert!(!r.stdout.contains("y.test.ts"), "{}", r.all());
    let r = tt(&p, &["gone.test.ts", "also-gone.test.ts"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        r.stderr,
        no_test_files(1, "filter: gone.test.ts, also-gone.test.ts\n")
    );
}

#[test]
fn subcommands_and_noop_flags_are_stripped() {
    let p = Project::new("launch-sub");
    p.file("a.test.ts", PASS);
    for sub in ["run", "watch", "dev"] {
        let r = tt(
            &p,
            &[
                sub,
                "--pool",
                "threads",
                "--sequence.seed",
                "1",
                "--globals",
                "--reporter",
                "json",
            ],
        );
        assert_eq!(r.code, Some(0), "{sub}: {}", r.all());
        assert_eq!(json_report(&r)["numTotalTestSuites"], 1);
        assert!(
            !r.stderr.contains("ignoring unsupported flag"),
            "{}",
            r.all()
        );
    }
}

#[test]
fn trailing_value_flag_without_value_keeps_the_test_file() {
    // turbo-test#32 item 3: `a.test.ts --reporter` printed the usage line (exit 2) because the
    // runner took the file as --reporter's value; with `-t` the path became the name pattern.
    let p = Project::new("launch-valueless");
    p.file("a.test.ts", PASS);
    for flag in ["--reporter", "-t", "--outputFile"] {
        let r = tt(&p, &["a.test.ts", flag]);
        assert_eq!(r.code, Some(0), "{flag}: {}", r.all());
        assert!(
            r.stdout.contains("PASS  a.test.ts  (1 passed, 0 failed)"),
            "{flag}: {}",
            r.all()
        );
        assert!(
            r.stderr
                .contains(&format!("turbo-test: ignoring '{flag}': missing value")),
            "{flag}: {}",
            r.all()
        );
    }
}

#[test]
fn environment_flag_and_config() {
    let p = Project::new("launch-env");
    p.file(
        "dom.test.ts",
        "test('dom', () => { expect(typeof document).toBe('object') })\n",
    );
    assert_eq!(tt(&p, &[]).code, Some(0));
    assert_eq!(tt(&p, &["--environment", "node"]).code, Some(1));
    assert_eq!(tt(&p, &["--environment=jsdom"]).code, Some(0));
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'node' } }\n",
    );
    assert_eq!(tt(&p, &[]).code, Some(1), "config environment applies");
    assert_eq!(
        tt(&p, &["--environment", "happy-dom"]).code,
        Some(0),
        "flag wins"
    );
}

#[test]
fn config_aliases_reach_the_resolver() {
    let p = Project::new("launch-alias");
    p.file("src/lib/value.ts", "export const value = 42\n");
    p.file("other/answer.ts", "export const answer = 'yes'\n");
    p.file(
        "vitest.config.ts",
        "import path from 'node:path'\n\
         export default {\n\
           resolve: { alias: { '@': path.resolve(__dirname, './src') } },\n\
           test: { alias: [ { find: '#other', replacement: './other' } ] },\n\
         }\n",
    );
    p.file(
        "a.test.ts",
        "import { value } from '@/lib/value'\nimport { answer } from '#other/answer'\n\
         test('alias', () => { expect(value).toBe(42); expect(answer).toBe('yes') })\n",
    );
    let r = tt(&p, &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("(1 passed, 0 failed)"), "{}", r.all());
}

#[test]
fn concatenated_dirname_alias_resolves() {
    // turbo-test#32 item 8a: `'@z': __dirname + '/z'` was dropped (`cannot resolve @z/m.js`)
    let p = Project::new("launch-alias-concat");
    p.file("z/m.js", "export const m = 'zed'\n");
    p.file(
        "vitest.config.js",
        "export default { resolve: { alias: { '@z': __dirname + '/z' } } };\n",
    );
    p.file(
        "a.test.js",
        "import { m } from '@z/m.js'\ntest('alias', () => { expect(m).toBe('zed') })\n",
    );
    let r = tt(&p, &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("(1 passed, 0 failed)"), "{}", r.all());
}

#[test]
fn slash_led_string_find_alias_resolves() {
    // turbo-test#32 item 8b: a quoted `find: '/@z'` was skipped as if it were a regex
    let p = Project::new("launch-alias-slash");
    p.file("z/m.js", "export const m = 'zed'\n");
    p.file(
        "vitest.config.js",
        "import path from 'node:path'\n\
         export default { resolve: { alias: [\n\
           { find: /^nope$/, replacement: path.resolve(__dirname, 'nope') },\n\
           { find: '/@z', replacement: path.resolve(__dirname, 'z') },\n\
         ] } };\n",
    );
    p.file(
        "a.test.js",
        "import { m } from '/@z/m.js'\ntest('alias', () => { expect(m).toBe('zed') })\n",
    );
    let r = tt(&p, &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("(1 passed, 0 failed)"), "{}", r.all());
}

#[test]
fn forced_config_and_root() {
    let p = Project::new("launch-config");
    p.file(
        "vitest.config.ts",
        "export default { test: { include: ['none/**'] } }\n",
    );
    p.file(
        "conf/ci.config.ts",
        "export default { test: { include: ['**/*.check.test.ts'] } }\n",
    );
    p.file("conf/x.check.test.ts", PASS);
    p.file("conf/y.test.ts", PASS);
    assert_eq!(tt(&p, &[]).code, Some(1), "root config includes nothing");
    for args in [
        &["-c", "conf/ci.config.ts"][..],
        &["--config=conf/ci.config.ts"],
    ] {
        let r = tt(&p, &[args, &["--reporter", "json"][..]].concat());
        assert_eq!(r.code, Some(0), "{args:?}: {}", r.all());
        assert_eq!(json_report(&r)["numTotalTestSuites"], 1);
        assert!(json_report(&r)["testResults"][0]["name"]
            .as_str()
            .unwrap()
            .ends_with("conf/x.check.test.ts"));
    }
    // --root/--dir move discovery (and the config search) into a subdirectory
    p.file(
        "pkg/vite.config.ts",
        "export default { test: { include: ['**/*.test.ts'] } }\n",
    );
    p.file("pkg/a.test.ts", PASS);
    p.file("pkg/sub/b.test.ts", PASS);
    for args in [&["--root", "pkg"][..], &["--dir=pkg"]] {
        let r = tt(&p, &[args, &["--reporter", "json"][..]].concat());
        assert_eq!(
            json_report(&r)["numTotalTestSuites"],
            2,
            "{args:?}: {}",
            r.all()
        );
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn changed_runs_only_changed_test_files() {
    let p = Project::new("launch-changed");
    p.file(".gitignore", ".tt-tmp/\n.tt-cache/\n");
    p.file("a.test.ts", PASS);
    p.file("b.test.ts", PASS);
    git(&p.dir, &["init", "-q"]);
    git(&p.dir, &["add", "."]);
    git(&p.dir, &["commit", "-q", "-m", "init"]);

    let r = tt(&p, &["--changed"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        r.stderr,
        "turbo-test: --changed: no changed test files (of 2) — exiting 0.\n"
    );

    p.file("b.test.ts", "test('b2', () => { expect(2).toBe(2) })\n");
    p.file("c.test.ts", PASS);
    let r = tt(&p, &["--changed", "--reporter", "json"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let names: Vec<_> = json_report(&r)["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(names, ["b.test.ts", "c.test.ts"]);
    let r = tt(
        &p,
        &[
            "--changed=HEAD",
            "a.test.ts",
            "b.test.ts",
            "--reporter",
            "json",
        ],
    );
    assert_eq!(json_report(&r)["numTotalTestSuites"], 1, "{}", r.all());

    // git missing from PATH → warn and run everything
    let r = turbo_test(
        &p.dir,
        &["--changed", "--reporter", "json"],
        &[("PATH", "/nonexistent")],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stderr.starts_with(
            "turbo-test: --changed: git unavailable / not a repo — running all discovered tests.\n"
        ),
        "{}",
        r.all()
    );
    assert_eq!(json_report(&r)["numTotalTestSuites"], 3);
}

#[test]
fn coverage_flags_and_config_thresholds() {
    let p = Project::new("launch-coverage");
    p.with_esbuild();
    p.file(
        "src/math.ts",
        "export function add(a: number, b: number) { return a + b }\nexport function unused(n: number) {\n  if (n > 1) { return n * 2 }\n  return n\n}\n",
    );
    p.file("src/skip.ts", "export const skipped = 1\n");
    p.file(
        "math.test.ts",
        "import { add } from './src/math'\nimport { skipped } from './src/skip'\ntest('add', () => { expect(add(1, 2) + skipped).toBe(4) })\n",
    );
    let summary = |dir: &str| -> serde_json::Value {
        let s = std::fs::read_to_string(p.path(&format!("{dir}/coverage-summary.json"))).unwrap();
        serde_json::from_str(&s).unwrap()
    };
    let files = |v: &serde_json::Value| -> Vec<String> {
        let mut k: Vec<String> = v
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| *k != "total")
            .map(|k| k.rsplit('/').next().unwrap().to_string())
            .collect();
        k.sort();
        k
    };

    // explicit flags: reporter, dir, include/exclude, per-file + a met threshold
    let r = tt(
        &p,
        &[
            "--coverage-reporter",
            "json-summary",
            "--coverage-dir",
            "cov1",
            "--coverage-include",
            "src/**",
            "--coverage-exclude",
            "src/skip.ts",
            "--coverage-per-file",
            "--coverage-thresholds",
            "lines=1",
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(files(&summary("cov1")), ["math.ts"]);

    // vitest config coverage block: thresholds/include injected when not passed
    p.file(
        "vitest.config.ts",
        "export default { test: { coverage: { include: ['src/**'], thresholds: { lines: 100 } } } }\n",
    );
    let r = tt(
        &p,
        &[
            "--coverage",
            "--coverage-reporters",
            "json-summary",
            "--coverage-dir",
            "cov2",
        ],
    );
    assert_eq!(
        r.code,
        Some(1),
        "unmet config threshold fails the run: {}",
        r.all()
    );
    assert_eq!(files(&summary("cov2")), ["math.ts", "skip.ts"]);
    // flags win over the config
    let r = tt(
        &p,
        &[
            "--coverage-threshold",
            "lines=1",
            "--coverage-reporter",
            "json-summary",
            "--coverage-dir",
            "cov3",
        ],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
}

/// turbo-test#45: a directory argument is a filter, as vitest's: every test file under it that
/// discovery would run, with the results of the same files passed one by one; a nested
/// directory works; a directory without test files is vitest's "No test files found, exiting
/// with code 1" (0 with --passWithNoTests). Checked against vitest 4.1.12.
#[test]
fn a_directory_argument_runs_the_test_files_under_it() {
    let p = Project::new("launch-dir-arg");
    let fail = "test('f', () => { expect(1).toBe(2) })\n";
    p.file("suite/a.test.ts", PASS);
    p.file("suite/b.spec.js", fail);
    p.file("suite/nested/c.test.ts", PASS);
    p.file("suite/nested/deeper/d.test.ts", PASS);
    p.file("suite/nested/helper.ts", "export const h = 1\n");
    p.file("elsewhere/e.test.ts", PASS);
    p.file("empty/sub/helper.ts", "export const h = 1\n");
    let root = format!("{}/", p.dir.display());
    let results = |args: &[&str]| {
        let mut a = vec!["--reporter", "json"];
        a.extend(args);
        let r = tt(&p, &a);
        let j = json_report(&r);
        let mut files: Vec<(String, String)> = j["testResults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                // the JSON report names files by absolute path, as vitest's
                let name = t["name"].as_str().unwrap();
                let rel = name.strip_prefix(&root).unwrap_or(name);
                (rel.to_string(), common::file_status(t))
            })
            .collect();
        files.sort();
        (r.code, files)
    };
    let explicit = results(&[
        "suite/a.test.ts",
        "suite/b.spec.js",
        "suite/nested/c.test.ts",
        "suite/nested/deeper/d.test.ts",
    ]);
    assert_eq!(explicit.1.len(), 4);
    assert_eq!(results(&["suite"]), explicit);
    assert_eq!(results(&["./suite/"]), explicit);
    let nested = results(&["suite/nested"]);
    assert_eq!(
        nested,
        (
            Some(0),
            vec![
                ("suite/nested/c.test.ts".to_string(), "passed".to_string()),
                (
                    "suite/nested/deeper/d.test.ts".to_string(),
                    "passed".to_string()
                ),
            ]
        )
    );
    // a directory and a file together
    assert_eq!(results(&["suite/nested", "elsewhere/e.test.ts"]).1.len(), 3);
    let r = tt(&p, &["empty"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(r.stderr, no_test_files(1, "filter: empty\n"));
    let r = tt(&p, &["empty", "--passWithNoTests"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stderr
            .starts_with("No test files found, exiting with code 0\n"),
        "{}",
        r.all()
    );
}

#[test]
fn root_names_files_relative_to_the_root() {
    // turbo-test#63 item 2: with --root, files were named relative to the working directory
    // (`sub/inner/a.test.js`, or absolute for a root above it). vitest 4.1.12 names them
    // relative to the root in every reporter; `--dir` (scan directory only) keeps the
    // working-directory-relative names.
    let p = Project::new("launch-root-names");
    p.file(
        "sub/inner/a.test.js",
        "test('ok', () => {}); test('bad', () => { expect(1).toBe(2) })\n",
    );
    p.file("sub/b.test.js", "test('ok2', () => {})\n");
    let lines = |r: &Run| -> Vec<String> {
        r.stdout
            .lines()
            .filter(|l| l.starts_with("PASS") || l.starts_with("FAIL"))
            .map(str::to_string)
            .collect()
    };
    let r = tt(&p, &["--root", "sub"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        lines(&r),
        [
            "PASS  b.test.js  (1 passed, 0 failed)",
            "FAIL  inner/a.test.js  (1 passed, 1 failed)"
        ],
        "{}",
        r.all()
    );
    // a positional filter is root-relative too
    let r = tt(&p, &["--root", "sub", "inner"]);
    assert_eq!(
        lines(&r),
        ["FAIL  inner/a.test.js  (1 passed, 1 failed)"],
        "{}",
        r.all()
    );
    let r = tt(&p, &["--root=sub", "--reporter", "junit"]);
    assert!(
        r.stdout.contains("<testsuite name=\"b.test.js\"")
            && r.stdout
                .contains("<testcase classname=\"inner/a.test.js\" name=\"bad\" time=\"")
            && r.stdout.contains("\n ❯ inner/a.test.js:1:53\n"),
        "{}",
        r.all()
    );
    // the JSON report keeps absolute names
    let r = tt(&p, &["--root", "sub", "--reporter", "json"]);
    assert_eq!(
        json_report(&r)["testResults"][0]["name"],
        p.path("sub/b.test.js").display().to_string()
    );
    // a root above the working directory, and --dir (the root stays the working directory)
    let r = turbo_test(&p.path("sub"), &["--root", ".."], &[]);
    assert_eq!(
        lines(&r),
        [
            "PASS  sub/b.test.js  (1 passed, 0 failed)",
            "FAIL  sub/inner/a.test.js  (1 passed, 1 failed)"
        ],
        "{}",
        r.all()
    );
    let r = tt(&p, &["--dir", "sub"]);
    assert_eq!(
        lines(&r),
        [
            "PASS  sub/b.test.js  (1 passed, 0 failed)",
            "FAIL  sub/inner/a.test.js  (1 passed, 1 failed)"
        ],
        "{}",
        r.all()
    );
}

#[test]
fn node_modules_and_git_are_skipped_only_by_the_effective_exclude() {
    // turbo-test#63 item 3: node_modules and .git were never searched, even when a config
    // `test.exclude` replaces vitest's default exclude. vitest 4.1.12 skips only what the
    // effective exclude lists (the files below are what it runs for each config).
    let p = Project::new("launch-exclude");
    for f in [
        "node_modules/foo/x.test.js",
        ".git/hooks/g.test.js",
        "src/s.test.js",
        "skip/k.test.js",
    ] {
        p.file(f, PASS);
    }
    let cases: [(Option<&str>, &[&str]); 5] = [
        (None, &["skip/k.test.js", "src/s.test.js"]),
        (
            Some("export default { test: { exclude: ['**/skip/**'] } }"),
            &[
                ".git/hooks/g.test.js",
                "node_modules/foo/x.test.js",
                "src/s.test.js",
            ],
        ),
        (
            Some("export default { test: { exclude: [] } }"),
            &[
                ".git/hooks/g.test.js",
                "node_modules/foo/x.test.js",
                "skip/k.test.js",
                "src/s.test.js",
            ],
        ),
        (
            Some("export default { test: { include: ['**/*.test.js'], exclude: ['**/skip/**'] } }"),
            &[
                ".git/hooks/g.test.js",
                "node_modules/foo/x.test.js",
                "src/s.test.js",
            ],
        ),
        (
            Some("export default { test: { include: ['**/*.test.js'] } }"),
            &["skip/k.test.js", "src/s.test.js"],
        ),
    ];
    let root = format!("{}/", p.dir.display());
    for (config, want) in cases {
        match config {
            Some(c) => {
                p.file("vitest.config.js", &format!("{c}\n"));
            }
            None => {
                let _ = std::fs::remove_file(p.path("vitest.config.js"));
            }
        }
        let r = tt(&p, &["--reporter", "json"]);
        assert_eq!(r.code, Some(0), "{config:?}: {}", r.all());
        let mut got: Vec<String> = json_report(&r)["testResults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().replace(&root, ""))
            .collect();
        got.sort();
        assert_eq!(got, want, "{config:?}: {}", r.all());
    }
}
