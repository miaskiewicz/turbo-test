//! `--help` / `-h` print usage and exit 0 without running the suite, `--version` / `-v` print the
//! version, and a vitest command turbo-test does not implement (`bench`, `list`, ...) is an error
//! rather than a run (turbo-test#427). Oracle: vitest 5.1.0 prints `vitest/5.1.0`, `Usage:` and
//! its options for `--help` / `-h` / `run --help` (exit 0) and `vitest/5.1.0` for `--version`.
#![cfg(unix)]
mod common;
use common::{turbo_test, Project};

/// A project whose only test writes `ran.txt`: whether the suite ran.
fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports = {};");
    p.file(
        "side.test.js",
        "require('node:fs').writeFileSync(__dirname + '/ran.txt', 'ran');\ntest('t', () => {});\n",
    );
    p
}

#[test]
fn help_prints_usage_and_does_not_run_the_suite() {
    let p = project("cli-help-427");
    let version = format!("turbo-test/{}\n", env!("CARGO_PKG_VERSION"));
    for args in [
        &["--help"][..],
        &["-h"],
        &["run", "--help"],
        &["side", "--reporter", "json", "-h"],
    ] {
        let r = turbo_test(&p.dir, args, &[]);
        assert_eq!(r.code, Some(0), "{args:?}: {}", r.all());
        assert!(
            r.stdout
                .starts_with(&format!("{version}\nUsage:\n  $ turbo-test [...filters]\n")),
            "{args:?}: {}",
            r.all()
        );
        for option in [
            "-h, --help",
            "-v, --version",
            "-t, --testNamePattern <pattern>",
            "--reporter <name>",
            "--coverage",
        ] {
            assert!(r.stdout.contains(option), "{option}: {}", r.stdout);
        }
        assert!(!p.path("ran.txt").exists(), "{args:?} ran the suite");
    }
    for args in [&["--version"][..], &["-v"]] {
        let r = turbo_test(&p.dir, args, &[]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        assert_eq!(r.stdout, version, "{}", r.all());
        assert!(!p.path("ran.txt").exists(), "{args:?} ran the suite");
    }
    // after `--`, `--help` is not an option (vitest/cac: an argument)
    let r = turbo_test(&p.dir, &["side", "--", "--help"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(p.path("ran.txt").exists(), "{}", r.all());
}

#[test]
fn unsupported_vitest_commands_fail_without_running_the_suite() {
    let p = project("cli-commands-427");
    for cmd in ["bench", "list", "related", "init", "doctor", "complete"] {
        let r = turbo_test(&p.dir, &[cmd], &[]);
        assert_eq!(r.code, Some(1), "{cmd}: {}", r.all());
        assert!(
            r.stderr
                .contains(&format!("turbo-test: `{cmd}` is not supported")),
            "{cmd}: {}",
            r.all()
        );
        assert!(r.stdout.is_empty(), "{cmd}: {}", r.all());
        assert!(!p.path("ran.txt").exists(), "{cmd} ran the suite");
    }
    // a supported command and an unknown word (a filter, as in vitest) still run
    let r = turbo_test(&p.dir, &["run", "side"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(p.path("ran.txt").exists(), "{}", r.all());
}
