//! TouchEvent list validation follows the file's environment (turbo-dom's TouchEventMode): jsdom
//! and happy-dom — and turbo-test's auto DOM, its jsdom-parity default — keep plain-object
//! entries in `touches` / `targetTouches` / `changedTouches` (`fireEvent.touchStart(el,
//! { touches: [{ clientX }] })`); the native `turbo-dom` environment is Chrome's (branded `Touch`
//! objects only). A reused isolate switches with each file's environment.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

const LENIENT: &str = r#"test('plain touch objects are kept', () => {
  const t = { clientX: 10, clientY: 20 };
  const e = new TouchEvent('touchstart', { touches: [t], targetTouches: [t], changedTouches: [t] });
  expect(e.touches.length).toBe(1);
  expect(e.touches[0]).toBe(t);
  expect(e.touches.item(0).clientX).toBe(10);
  expect([...e.changedTouches]).toEqual([t]);
  expect(e.targetTouches[0].clientY).toBe(20);
});
"#;

const STRICT: &str = r#"test('Chrome requires Touch objects', () => {
  expect(() => new TouchEvent('touchstart', { touches: [{ clientX: 10 }] })).toThrow(TypeError);
});
"#;

#[test]
fn touch_event_lists_follow_the_files_environment() {
    for reuse in [false, true] {
        let p = Project::new(&format!("touch-mode-{reuse}"));
        p.file("package.json", r#"{ "name": "touch-mode" }"#);
        p.file("vitest.config.js", "export default { test: {} };\n");
        p.file(
            "a_jsdom.test.js",
            &format!("// @vitest-environment jsdom\n{LENIENT}"),
        );
        p.file(
            "b_strict.test.js",
            &format!("// @vitest-environment turbo-dom\n{STRICT}"),
        );
        p.file(
            "c_happy.test.js",
            &format!("// @vitest-environment happy-dom\n{LENIENT}"),
        );
        // no environment: turbo-test's auto DOM (it mentions `document`)
        p.file("d_auto.test.js", &format!("document.body;\n{LENIENT}"));
        let env: &[(&str, &str)] = if reuse {
            &[("TURBO_REUSE_ISOLATE", "1")]
        } else {
            &[]
        };
        let r = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "1"], env);
        let j = json_report(&r);
        assert_eq!(
            (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
            (Some(4), Some(0)),
            "reuse={reuse}:\n{}",
            r.all()
        );
    }
}
