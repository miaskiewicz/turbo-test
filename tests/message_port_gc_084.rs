//! Pending messages retain their receiving endpoint until delivery (React act enqueueTask).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn ephemeral_message_channel_survives_collection_before_delivery() {
    let p = Project::new("message-port-pending-gc");
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"message-port-pending-gc\"}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "port.test.cjs",
        r#"
const assert = require('node:assert/strict');
test('a short-lived channel delivers through a preceding collection task', async () => {
  assert.equal(typeof gc, 'function');
  let delivered = 0;
  await new Promise(resolve => {
    setImmediate(() => { gc(); gc(); });
    (() => {
      const channel = new MessageChannel();
      channel.port1.onmessage = function () { delivered++; this.close(); resolve(); };
      channel.port2.unref();
      channel.port2.postMessage('pending');
    })();
  });
  assert.equal(delivered, 1);
});
"#,
    );
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
