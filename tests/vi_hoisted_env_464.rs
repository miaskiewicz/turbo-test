//! `vi.hoisted(...)` must run BEFORE any ES static-import of the test file is evaluated —
//! matching vitest — so a hoisted block can seed `process.env` (or any other global) for a
//! module that reads it at load time (turbo-test#464).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn hoisted_runs_before_static_import_evaluates() {
    let p = Project::new("vi-hoisted-env-464");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"r","version":"0.0.0","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "import { defineConfig } from 'vitest/config';\n\
         export default defineConfig({ test: { environment: 'node' } });\n",
    );
    // Dependency reads process.env at module-load time.
    p.file(
        "test/env-dep.ts",
        "export const AT_LOAD = process.env.TT_HOISTED_464 ?? 'unset';\n",
    );
    // Entry statically imports the dep. The vi.hoisted() callback seeds process.env;
    // under vitest it runs before the static import graph is evaluated, so AT_LOAD = 'set'.
    p.file(
        "test/hoisted-env.test.ts",
        "import { expect, it, vi } from 'vitest';\n\
         vi.hoisted(() => { process.env.TT_HOISTED_464 = 'set'; });\n\
         import { AT_LOAD } from './env-dep';\n\
         it('vi.hoisted runs before a static import reads the env at module load', () => {\n\
         \texpect(AT_LOAD).toBe('set');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    let passed = j["numPassedTests"].as_u64().unwrap_or(0);
    let failed = j["numFailedTests"].as_u64().unwrap_or(0);
    assert_eq!((passed, failed), (1, 0), "{}", r.all());
}
