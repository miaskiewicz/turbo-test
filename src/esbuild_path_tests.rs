//! Platform-specific npm esbuild entry-point discovery (#278).
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "turbo-esbuild-path-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        Self(root)
    }
    fn add(&self, name: &str) -> PathBuf {
        let path = self.0.join("node_modules/.bin").join(name);
        std::fs::write(&path, "").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn missing_local_esbuild_remains_missing() {
    let fixture = Fixture::new();
    assert_eq!(local(&fixture.0), None);
}

#[cfg(windows)]
#[test]
fn windows_prefers_cmd_then_exe_then_extensionless() {
    let fixture = Fixture::new();
    let plain = fixture.add("esbuild");
    let exe = fixture.add("esbuild.exe");
    let cmd = fixture.add("esbuild.cmd");
    assert_eq!(local(&fixture.0), Some(cmd.clone()));
    std::fs::remove_file(cmd).unwrap();
    assert_eq!(local(&fixture.0), Some(exe.clone()));
    std::fs::remove_file(exe).unwrap();
    assert_eq!(local(&fixture.0), Some(plain));
}

#[cfg(not(windows))]
#[test]
fn unix_ignores_windows_wrappers_and_keeps_extensionless_priority() {
    let fixture = Fixture::new();
    fixture.add("esbuild.cmd");
    fixture.add("esbuild.exe");
    assert_eq!(local(&fixture.0), None);
    let plain = fixture.add("esbuild");
    assert_eq!(local(&fixture.0), Some(plain));
}

#[cfg(windows)]
#[test]
fn windows_executes_the_installed_npm_cmd_and_transforms_typescript() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let executable = local(root).expect("npm ci must install the Windows esbuild entry point");
    assert_eq!(executable.file_name().unwrap(), "esbuild.cmd");
    let version = Command::new(&executable).arg("--version").output().unwrap();
    assert!(
        version.status.success(),
        "{}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert!(
        !version.stdout.is_empty(),
        "installed esbuild must report its version"
    );
    let mut child = Command::new(&executable)
        .args(["--loader=ts", "--format=cjs", "--log-level=error"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"const value: number = 42; module.exports = value;")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let javascript = String::from_utf8(output.stdout).unwrap();
    assert!(!javascript.contains(": number"), "{javascript}");
    let result = Command::new("node").arg("-e")
        .arg("const module = { exports: undefined }; new Function('module', process.argv[1])(module); process.stdout.write(String(module.exports));")
        .arg(javascript).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"42");
}
