fn main() {
    // Export the binary's dynamic symbols so dlopen'd native (.node) addons can resolve
    // their undefined `napi_*` imports against our Node-API host implementations.
    println!("cargo:rustc-link-arg-bins=-Wl,-export_dynamic");
}
