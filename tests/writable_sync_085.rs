//! node:stream Writable writes synchronously like Node, and process.pid / process.ppid are the
//! host's real ids. Expectations come from real Node v26.11.0; the data files also run under
//! `node --test`.
#![cfg(unix)]
mod common;
use common::{command, json_report, turbo_test, Project, Run};

fn node_project(name: &str, file: &str, source: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(file, source);
    p
}

#[test]
fn writable_write_cork_writev_drain_and_error_orderings_match_node() {
    let p = node_project(
        "writable-sync-085",
        "writable.test.cjs",
        include_str!("data/compat-085/writable-sync.cjs"),
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 17, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn process_pid_and_ppid_are_the_host_process_ids() {
    let p = node_project(
        "process-ids-085",
        "ids.test.cjs",
        include_str!("data/compat-085/process-ids.cjs"),
    );
    let out = p.path("ids.json");
    let ids_out = out.to_str().unwrap();
    let exe = env!("CARGO_BIN_EXE_turbo-test");
    let child = command(
        exe,
        &p.dir,
        &["--reporter", "json"],
        &[("IDS_OUT", ids_out)],
        &[],
    )
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .unwrap();
    let pid = child.id();
    let output = child.wait_with_output().unwrap();
    let run = Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    let ids: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(ids["pid"], pid, "{}", run.all());
    assert_eq!(ids["ppid"], std::process::id(), "{}", run.all());
}
