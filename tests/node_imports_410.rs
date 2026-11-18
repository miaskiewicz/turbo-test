//! Reported Node API imports must work without enabling Jest compatibility (#410).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn vitest_node_imports_expose_webcrypto_child_process_and_buffer() {
    let p = Project::new("node-imports-410");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"node-imports-410","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default {test:{environment:'node'}};",
    );
    p.file("imports.test.ts", r#"
import { execFileSync } from 'node:child_process';
import { Buffer as ImportedBuffer } from 'node:buffer';
import { webcrypto, subtle } from 'node:crypto';
import { expect, it } from 'vitest';
it('global WebCrypto digest', async () => {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode('x'));
  expect(Buffer.from(digest).toString('hex')).toBe('2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881');
  expect(webcrypto.subtle).toBe(crypto.subtle);
  expect(subtle).toBe(crypto.subtle);
});
it('named child process export executes real Node', () => {
  expect(execFileSync(process.execPath, ['-e', 'process.stdout.write("ok")'], {encoding:'utf8'})).toBe('ok');
});
it('global and imported Buffer expose byteLength', () => {
  expect(Buffer.byteLength('é')).toBe(2);
  expect(ImportedBuffer.byteLength('é')).toBe(2);
});
"#);
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
