//! TouchEvent list validation and computed `text-decoration` follow the file's environment
//! (turbo-dom's TouchEventMode / ComputedStyleMode): jsdom and happy-dom — and turbo-test's auto
//! DOM, its jsdom-parity default — keep plain-object entries in `touches` / `targetTouches` /
//! `changedTouches` (`fireEvent.touchStart(el, { touches: [{ clientX }] })`) and report the
//! declared `text-decoration` (`line-through`, as jest-dom's `toHaveStyle` reads it); the native
//! `turbo-dom` environment is Chrome's (branded `Touch` objects only, `line-through
//! rgb(0, 0, 0)`). A reused isolate switches with each file's environment.
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
test('computed text-decoration is the declared shorthand', () => {
  const s = document.createElement('span');
  s.style.textDecoration = 'line-through';
  document.body.appendChild(s);
  expect(getComputedStyle(s).textDecoration).toBe('line-through');
  s.style.textDecoration = 'underline red';
  expect(getComputedStyle(s).textDecoration).toBe('underline red');
  // agrees with the inline serialization (jest-dom builds its expected value through a style
  // declaration) and follows a change of color format only
  s.style.textDecoration = 'underline #161925';
  expect(getComputedStyle(s).textDecoration).toBe(s.style.textDecoration);
  s.style.textDecoration = 'underline rgb(1, 2, 3)';
  expect(getComputedStyle(s).textDecoration).toBe(s.style.textDecoration);
  expect(s.style.textDecoration).toBe('underline rgb(1, 2, 3)');
});
"#;

const STRICT: &str = r#"test('Chrome requires Touch objects', () => {
  expect(() => new TouchEvent('touchstart', { touches: [{ clientX: 10 }] })).toThrow(TypeError);
});
test('Chrome serializes the computed text-decoration with its color', () => {
  const s = document.createElement('span');
  s.style.textDecoration = 'line-through';
  document.body.appendChild(s);
  expect(getComputedStyle(s).textDecoration).toBe('line-through rgb(0, 0, 0)');
});
"#;

#[test]
fn jsdom_compat_modes_follow_the_files_environment() {
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
            (Some(8), Some(0)),
            "reuse={reuse}:\n{}",
            r.all()
        );
    }
}
