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
