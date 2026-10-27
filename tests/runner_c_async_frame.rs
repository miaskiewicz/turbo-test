//! An anonymous async frame (`at async /abs/file.js:L:C`) is a location (turbo-test#169).
//! Recorded from vitest 4.1.12: an error made after an await in a helper prints the helper's
//! frame with its code frame, then the test's own (anonymous, suspended) frame.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

#[test]
#[ignore = "turbo-test#169: fails until the fix"]
fn an_error_made_after_an_await_keeps_the_test_file_frame() {
    let p = Project::new("rc-async-frame");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "helper.js",
        "export async function late() {\n  await null\n  throw new Error(\"late\")\n}\n",
    );
    p.file(
        "afail.test.js",
        "import { test } from 'vitest'\nimport { late } from './helper.js'\ntest('after await', async () => {\n  await late()\n})\n",
    );
    let r = turbo_test(
        &p.dir,
        &["afail.test.js"],
        &[("TURBO_LEGACY_REPORTER", "0"), ("NO_COLOR", "1")],
    );
    let vitest = " FAIL  afail.test.js > after await
Error: late
 ❯ late helper.js:3:9
      1| export async function late() {
      2|   await null
      3|   throw new Error(\"late\")
       |         ^
      4| }
      5|
 ❯ afail.test.js:4:3

⎯⎯⎯⎯";
    assert!(r.all().contains(vitest), "{}", r.all());
    assert_eq!(r.code, Some(1), "{}", r.all());
}
