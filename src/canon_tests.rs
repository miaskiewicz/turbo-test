//! Verbatim-prefix stripping on Windows-style path strings (they are plain strings on every
//! platform, so this runs on each CI leg), and `canonicalize` on a real path of this platform.

use super::*;

fn strip(s: &str) -> String {
    strip_windows_verbatim(PathBuf::from(s))
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_verbatim_drive_path_loses_its_prefix() {
    assert_eq!(strip(r"\\?\C:\proj\src\a.ts"), r"C:\proj\src\a.ts");
    assert_eq!(strip(r"\\?\d:\x"), r"d:\x");
    assert_eq!(strip(r"\\?\C:\"), r"C:\");
    assert_eq!(strip(r"\\?\C:"), r"C:");
    // a pnpm virtual-store path keeps everything after the prefix
    assert_eq!(
        strip(r"\\?\C:\repo\node_modules\.pnpm\tslib@2.8.1\node_modules\tslib\tslib.js"),
        r"C:\repo\node_modules\.pnpm\tslib@2.8.1\node_modules\tslib\tslib.js"
    );
}

#[test]
fn a_verbatim_unc_path_becomes_a_unc_path() {
    assert_eq!(strip(r"\\?\UNC\srv\share\a.ts"), r"\\srv\share\a.ts");
    assert_eq!(strip(r"\\?\UNC\srv\share"), r"\\srv\share");
}

#[test]
fn other_paths_are_unchanged() {
    for s in [
        r"C:\proj\a.ts",
        r"\\srv\share\a.ts",
        r"\\?\Volume{0b1c}\a.ts",
        r"\\?\GLOBALROOT\Device\x",
        r"\\?\1:\x",
        r"\\?\CC:\x",
        r"\\?\C:x",
        r"\\.\C:\x",
        "/usr/lib/a.js",
        "rel/a.js",
        "",
    ] {
        assert_eq!(strip(s), s, "{s}");
    }
}

#[cfg(unix)]
#[test]
fn a_non_utf8_path_is_unchanged() {
    use std::os::unix::ffi::OsStrExt;
    let p = PathBuf::from(std::ffi::OsStr::from_bytes(b"\\\\?\\C:\\\xff"));
    assert_eq!(strip_windows_verbatim(p.clone()), p);
    assert_eq!(strip_verbatim(p.clone()), p);
}

#[test]
fn canonicalize_resolves_without_a_verbatim_prefix() {
    let dir = std::env::temp_dir().join(format!("turbo-test-canon-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let got = canonicalize(dir.join("sub").join("..")).unwrap();
    assert!(!got.to_string_lossy().starts_with(r"\\?\"), "{got:?}");
    assert_eq!(got, strip_verbatim(std::fs::canonicalize(&dir).unwrap()));
    assert!(got.is_absolute() && got.is_dir(), "{got:?}");
    assert!(canonicalize(dir.join("missing")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every canonical path the runner hands on is [`canonicalize`]'s: no other source file calls
/// `std::fs::canonicalize` / `Path::canonicalize` (their tests may, for scratch directories).
#[test]
fn the_runner_canonicalizes_only_through_this_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if skip_canon_scan(&name) {
            continue;
        }
        collect_canon_offenders(&path, &name, &mut offenders);
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}

fn skip_canon_scan(name: &str) -> bool {
    !name.ends_with(".rs") || name.contains("_tests") || name.starts_with("canon")
}

fn collect_canon_offenders(path: &Path, name: &str, offenders: &mut Vec<String>) {
    let text = std::fs::read_to_string(path).unwrap();
    // a file's own `mod tests { … }` block is test code too
    let text = text.split("#[cfg(test)]\nmod tests {").next().unwrap();
    for (i, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap();
        if code.contains("fs::canonicalize(") || code.contains(".canonicalize()") {
            offenders.push(format!("{name}:{}: {}", i + 1, line.trim()));
        }
    }
}

#[cfg(not(windows))]
#[test]
fn production_prefix_normalization_preserves_unix_literal_names() {
    for name in [r"\\?\C:\literal.ts", r"\\?\UNC\server\share\literal.ts"] {
        let path = PathBuf::from(name);
        assert_eq!(strip_verbatim(path.clone()), path);
    }
}

#[cfg(windows)]
#[test]
fn windows_canonical_paths_open_the_same_file_without_the_prefix() {
    let dir = std::env::temp_dir().join(format!("turbo-canon-windows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fixture.js");
    std::fs::write(&file, "export const answer = 42;").unwrap();
    let native = std::fs::canonicalize(&file).unwrap();
    assert!(native.to_string_lossy().starts_with(r"\\?\"));
    let normalized = canonicalize(&file).unwrap();
    assert!(!normalized.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        std::fs::read(normalized).unwrap(),
        std::fs::read(native).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
