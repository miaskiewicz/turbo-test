//! A DOM-environment file can use Node's backend built-ins (tty, zlib, child_process, crypto
//! ciphers), as under vitest's jsdom: the backend payload is activated on first use instead of
//! throwing (regression since 0.8.4 / #390; @playwright/test requires tty at load). The data file
//! also passes under `node --test`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn dom_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p.file(
        "vitest.config.mjs",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    p.file(
        "backend.test.cjs",
        include_str!("data/compat-085/dom-backend-builtins.cjs"),
    );
    p
}

#[test]
fn a_dom_file_uses_backend_builtins_like_vitest_jsdom() {
    let p = dom_project("dom-backend-085");
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn backend_activation_in_a_reused_dom_context_keeps_later_files_working() {
    let p = dom_project("dom-backend-reuse-085");
    p.file(
        "later.test.cjs",
        "test('a later DOM file still runs', () => { expect(document.createElement('div').tagName).toBe('DIV'); expect(require('zlib').inflateSync(require('zlib').deflateSync(Buffer.from('x'))).toString()).toBe('x'); });\n",
    );
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--jobs", "1", "--no-isolate"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 4, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
