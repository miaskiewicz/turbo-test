//! #410: top-level-await setup bundles keep the source file's import resolution context.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn project(name: &str, setup: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"setup-dynamic","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'node', setupFiles: ['./test/setup.ts'] } };\n",
    );
    p.file("test/setup.ts", setup);
    p.file("test/value.ts", "export const value = 'relative';\n");
    p.file(
        "node_modules/setup-package/package.json",
        r#"{"name":"setup-package","main":"index.cjs"}"#,
    );
    p.file(
        "node_modules/setup-package/index.cjs",
        "exports.value = 'package';\n",
    );
    p.file("test/a.test.ts", "import { expect, it } from 'vitest';\nit('setup import reaches this runtime', () => expect(globalThis.SETUP_VALUE).toBe('ready'));\n");
    p
}

fn passes(p: &Project) {
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(json_report(&r)["numPassedTests"], 1, "{}", r.all());
}

#[test]
fn setup_top_level_await_imports_package_from_its_source_directory() {
    let p = project("setup-dynamic-package-410", "const { value } = await import('setup-package');\nif (value !== 'package') throw Error('wrong package');\nglobalThis.SETUP_VALUE = 'ready';\n");
    passes(&p);
    // The cached bundle must retain the same source directory on the next invocation.
    passes(&p);
}

#[test]
fn setup_runtime_relative_import_uses_the_nested_setup_directory() {
    let p = project("setup-dynamic-relative-410", "const spec = './value.ts';\nconst { value } = await import(spec);\nif (value !== 'relative') throw Error('wrong relative module');\nglobalThis.SETUP_VALUE = 'ready';\n");
    passes(&p);
}

#[test]
fn setup_absolute_package_import_keeps_its_vitest_require_external() {
    let p = project("setup-dynamic-runtime-410", "");
    // Like jest-dom/vitest: the package's CJS entry extends this worker's expect. Bundling
    // its require('vitest') into an ESM setup bundle instead creates an unsupported require.
    p.file(
        "node_modules/setup-matchers/package.json",
        r#"{"name":"setup-matchers","main":"index.cjs"}"#,
    );
    let package = p.file("node_modules/setup-matchers/index.cjs", "const { expect } = require('vitest');\nexpect.extend({ toBeReady(value) { return { pass: value === 'ready', message: () => 'expected ready' }; } });\n");
    p.file(
        "test/setup.ts",
        &format!(
            "await import({});\nglobalThis.SETUP_VALUE = 'ready';\n",
            serde_json::to_string(&package.to_string_lossy()).unwrap()
        ),
    );
    p.file("test/a.test.ts", "import { expect, it } from 'vitest';\nit('package extends the same expect', () => expect(globalThis.SETUP_VALUE).toBeReady());\n");
    passes(&p);
}
