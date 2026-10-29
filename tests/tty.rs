//! A run whose stdout is a terminal: the test files see `process.env.FORCE_TTY === 'true'`, as
//! vitest 4.1.12 sets it for its workers (checked with `script -q /dev/null npx vitest run`).
#![cfg(unix)]

mod common;
use common::Project;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::process::Stdio;

const FORCE_TTY: &str = "import { test, expect } from 'vitest'\n\
                         test('tty', () => { expect(process.env.FORCE_TTY).toBe('true') })\n";

#[test]
fn a_terminal_stdout_sets_force_tty() {
    let p = Project::new("tty-force");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("t.test.js", FORCE_TTY);
    let (mut master, mut slave) = (0, 0);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty");
    let (master, slave) = unsafe {
        (
            std::fs::File::from_raw_fd(master),
            std::fs::File::from_raw_fd(slave),
        )
    };
    let mut child = common::command(env!("CARGO_BIN_EXE_turbo-test"), &p.dir, &[], &[], &[])
        .stdout(Stdio::from(slave))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // the terminal's output, read until the run closes it (EIO on macOS)
    let reader = std::thread::spawn(move || {
        let mut master = master;
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = master.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&out).into_owned()
    });
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let out = reader.join().unwrap();
    assert!(out.contains("1 passed"), "{out}\n{stderr}");
    assert_eq!(status.code(), Some(0), "{out}\n{stderr}");
    // piped: no FORCE_TTY, the same test fails
    let r = common::turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
}
