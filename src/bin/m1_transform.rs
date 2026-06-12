//! M1.4 — TS/JSX transform via oxc (the transform substrate, spec §3/§M4).
//!
//! Strips TypeScript types and lowers TS-only constructs (enums, etc.) to plain JS the
//! V8 module loader can compile. This is the `transform()` hook the ESM/CJS loaders call
//! before handing source to V8. oxc is reused as-is per the spec (don't reimplement).

use std::path::Path;

use turbo_test::transform::transform;

fn main() {
    let file = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/ts/sample.ts".to_string());
    let path = Path::new(&file);
    let src = std::fs::read_to_string(path).expect("read source");

    println!("turbo-test M1.4 — oxc TS/JSX transform");
    println!("input: {file}\n--- transformed ---");
    match transform(path, &src) {
        Ok(code) => {
            println!("{code}");
            // sanity: TS-only syntax must be gone, exports preserved
            let ok = !code.contains("interface ")
                && !code.contains(": Greeting")
                && code.contains("export");
            println!("--- check ---");
            println!("{}", if ok { "==> M1.4 TRANSFORM PASS" } else { "==> M1.4 FAIL" });
            std::process::exit(if ok { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("transform failed: {e}");
            std::process::exit(1);
        }
    }
}
