//! Date constructor metadata must remain usable for schema libraries.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn virtual_and_fake_date_keep_the_native_constructor_brand() {
    let p = Project::new("date-brand-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"date-brand-084"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    let fixture = p.file(
        "date.test.cjs",
        include_str!("data/compat-084/date-brand.cjs"),
    );
    let node = std::process::Command::new("node")
        .arg("--test")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}{}",
        String::from_utf8_lossy(&node.stdout),
        String::from_utf8_lossy(&node.stderr)
    );
    p.file("fake.test.cjs",r#"
const assert = require('node:assert/strict');
test('fake Date name and clock remain stable through restore', () => {
 const original=Date;
 try {
   vi.useFakeTimers(); vi.setSystemTime(new original('2024-03-01T00:00:00.000Z'));
   assert.equal(Date.name,'Date'); assert.equal(new Date().toISOString(),'2024-03-01T00:00:00.000Z');
   vi.advanceTimersByTime(1000); assert.equal(Date.now(),original.parse('2024-03-01T00:00:01.000Z'));
   assert.equal(Object.prototype.toString.call(new Date()),'[object Date]');
 } finally { vi.useRealTimers(); }
 assert.equal(Date,original); assert.equal(Date.name,'Date');
});
"#);
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
