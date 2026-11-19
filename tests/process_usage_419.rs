//! process.uptime / memoryUsage / cpuUsage / resourceUsage / umask (turbo-test#419): real host
//! figures in Node's shapes and units (getrusage microseconds, maxRSS in kilobytes, V8 heap
//! statistics, the resident set), cpuUsage(prev) differences and Node's argument errors.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
const fs = require('fs');
const os = require('os');
const path = require('path');
const codeOf = (f) => { try { f(); return 'no error'; } catch (e) { return e.name + ' ' + e.code; } };
const spin = (ms) => { const end = Date.now() + ms; let x = 0; while (Date.now() < end) x++; return x; };
test('process.uptime is the seconds this process has run and advances with time', async () => {
 expect(typeof process.uptime).toBe('function');
 const a = process.uptime();
 expect(a).toBeGreaterThan(0);
 expect(a).toBeLessThan(24 * 3600);
 await new Promise((r) => setTimeout(r, 60));
 expect(process.uptime() - a).toBeGreaterThanOrEqual(0.05);
});
test('process.memoryUsage reports the host heap and resident set in Node\'s shape', () => {
 const m = process.memoryUsage();
 expect(Object.keys(m)).toEqual(['rss', 'heapTotal', 'heapUsed', 'external', 'arrayBuffers']);
 for (const k of Object.keys(m)) expect([k, Number.isInteger(m[k]) && m[k] >= 0]).toEqual([k, true]);
 expect(m.heapUsed).toBeLessThanOrEqual(m.heapTotal);
 expect(m.heapUsed).toBeGreaterThan(1 << 20);
 expect(m.arrayBuffers).toBeLessThanOrEqual(m.external);
 expect(m.rss).toBeGreaterThan(m.heapUsed);
 const rss = process.memoryUsage.rss();
 expect(Number.isInteger(rss) && rss > (1 << 22)).toBe(true);
 const held = Buffer.alloc(64 << 20, 1);
 const after = process.memoryUsage();
 expect(after.arrayBuffers - m.arrayBuffers).toBeGreaterThanOrEqual(48 << 20);
 expect(after.external - m.external).toBeGreaterThanOrEqual(48 << 20);
 expect(after.rss - m.rss).toBeGreaterThanOrEqual(32 << 20);
 expect(held[held.length - 1]).toBe(1);
});
test('process.cpuUsage counts this process\'s CPU time and diffs against a previous value', () => {
 const a = process.cpuUsage();
 expect(Object.keys(a)).toEqual(['user', 'system']);
 expect(Number.isInteger(a.user) && a.user > 0 && Number.isInteger(a.system) && a.system >= 0).toBe(true);
 spin(80);
 const d = process.cpuUsage(a);
 expect(d.user + d.system).toBeGreaterThanOrEqual(20000);
 const b = process.cpuUsage();
 expect(b.user - a.user).toBeGreaterThanOrEqual(d.user);
 expect(process.cpuUsage(null).user).toBeGreaterThanOrEqual(b.user);
 const huge = process.cpuUsage({ user: Number.MAX_SAFE_INTEGER, system: 0 });
 expect(huge.user).toBeLessThan(0);
 expect([5, {}, { user: 1 }, { user: '1', system: 2 }, { user: -1, system: 2 }, { user: NaN, system: 1 }, { user: 1, system: 1e30 }].map((p) => codeOf(() => process.cpuUsage(p))))
  .toEqual(['TypeError ERR_INVALID_ARG_TYPE', 'TypeError ERR_INVALID_ARG_TYPE', 'TypeError ERR_INVALID_ARG_TYPE', 'TypeError ERR_INVALID_ARG_TYPE', 'RangeError ERR_INVALID_ARG_VALUE', 'RangeError ERR_INVALID_ARG_VALUE', 'RangeError ERR_INVALID_ARG_VALUE']);
});
test('process.resourceUsage is getrusage in Node\'s field order and units', () => {
 const cpu = process.cpuUsage();
 const r = process.resourceUsage();
 expect(Object.keys(r)).toEqual(['userCPUTime', 'systemCPUTime', 'maxRSS', 'sharedMemorySize', 'unsharedDataSize', 'unsharedStackSize', 'minorPageFault', 'majorPageFault', 'swappedOut', 'fsRead', 'fsWrite', 'ipcSent', 'ipcReceived', 'signalsCount', 'voluntaryContextSwitches', 'involuntaryContextSwitches']);
 for (const k of Object.keys(r)) expect([k, Number.isInteger(r[k]) && r[k] >= 0]).toEqual([k, true]);
 expect(r.userCPUTime).toBeGreaterThanOrEqual(cpu.user);
 expect(r.systemCPUTime).toBeGreaterThanOrEqual(cpu.system);
 expect(r.minorPageFault).toBeGreaterThan(0);
 // maxRSS is the peak resident set in kilobytes
 const rss = process.memoryUsage.rss();
 expect(r.maxRSS * 1024).toBeGreaterThanOrEqual(rss * 0.9);
 expect(r.maxRSS * 1024).toBeLessThan(rss * 64);
});
test('process.umask reads and sets the file mode creation mask', () => {
 const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'umask-'));
 const original = process.umask(0o077);
 try {
  expect(Number.isInteger(original)).toBe(true);
  expect(process.umask()).toBe(0o077);
  expect(process.umask(undefined)).toBe(0o077);
  fs.writeFileSync(path.join(dir, 'private'), 'x');
  expect(fs.statSync(path.join(dir, 'private')).mode & 0o777).toBe(0o600);
  expect(process.umask('022')).toBe(0o077);
  expect(process.umask()).toBe(0o022);
  fs.writeFileSync(path.join(dir, 'shared'), 'x');
  expect(fs.statSync(path.join(dir, 'shared')).mode & 0o777).toBe(0o644);
  expect([null, {}, '0o22', 'abc', -1, 1.5, 2 ** 32].map((m) => codeOf(() => process.umask(m))))
   .toEqual(['TypeError ERR_INVALID_ARG_TYPE', 'TypeError ERR_INVALID_ARG_TYPE', 'TypeError ERR_INVALID_ARG_VALUE', 'TypeError ERR_INVALID_ARG_VALUE', 'RangeError ERR_OUT_OF_RANGE', 'RangeError ERR_OUT_OF_RANGE', 'RangeError ERR_OUT_OF_RANGE']);
  expect(process.umask()).toBe(0o022);
 } finally {
  process.umask(original);
  fs.rmSync(dir, { recursive: true, force: true });
 }
});
"#;
#[test]
fn process_usage_apis_report_host_figures_like_node() {
    let p = Project::new("process-usage-419");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("process.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
