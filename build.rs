fn main() {
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
