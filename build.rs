/// A fingerprint of everything that shapes the runner's transforms (the sources and the locked
/// dependency versions): the default transform cache is per build (turbo-test#207).
fn build_id() -> String {
    use std::hash::{Hash, Hasher};
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(std::path::Path::new("src"), &mut files);
    files.push("Cargo.lock".into());
    files.push("build.rs".into());
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &files {
        f.hash(&mut h);
        std::fs::read(f).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=TT_BUILD_ID={}", build_id());
    // Export the binary's dynamic symbols so dlopen'd native (.node) addons can resolve
    // their undefined `napi_*` imports against our Node-API host implementations.
    //
    // The flag spelling is linker-specific and is NOT interchangeable:
    //   - macOS ld64:    -export_dynamic   (single dash, underscore)
    //   - GNU ld / lld:  --export-dynamic  (double dash, hyphens)
    //
    // Passing the macOS spelling to GNU ld is silently misparsed as `-e xport_dynamic`
    // (`-e` sets the entry symbol). `xport_dynamic` is undefined, so the linker emits a
    // binary with e_entry = 0 — which segfaults inside ld.so at startup, before main()
    // ever runs. (This shipped broken in the linux-x64 prebuilt through v0.2.3.)
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-export_dynamic");
        }
        "windows" => {
            // MSVC link.exe has no equivalent flag; Node-API symbol resolution on
            // Windows works differently, so emit nothing here.
        }
        _ => {
            // Linux and other GNU-ld / lld-linked unixes.
            println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic");
        }
    }
}
