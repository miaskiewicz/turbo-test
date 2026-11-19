// process.pid / process.ppid are the host process's real ids, shaped as Node v26.11.0 defines
// them: own enumerable data properties, pid read-only, ppid writable. IDS_OUT names a file the
// ids are written to, for the harness to compare with the process it spawned.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');

const shape = (key) => {
  const d = Object.getOwnPropertyDescriptor(process, key);
  return { type: typeof d.value, writable: d.writable, enumerable: d.enumerable, configurable: d.configurable, accessor: Boolean(d.get || d.set) };
};

testCase('pid and ppid are positive integers and differ', () => {
  for (const id of [process.pid, process.ppid]) assert.ok(Number.isInteger(id) && id > 0, String(id));
  assert.notEqual(process.pid, process.ppid);
  assert.equal(require('node:process').pid, process.pid);
  if (process.env.IDS_OUT) fs.writeFileSync(process.env.IDS_OUT, JSON.stringify({ pid: process.pid, ppid: process.ppid }));
});

testCase('pid is a read-only data property and ppid a writable one', () => {
  assert.deepEqual(shape('pid'), { type: 'number', writable: false, enumerable: true, configurable: true, accessor: false });
  assert.deepEqual(shape('ppid'), { type: 'number', writable: true, enumerable: true, configurable: true, accessor: false });
  const pid = process.pid;
  const ppid = process.ppid;
  process.pid = 1;
  assert.equal(process.pid, pid);
  process.ppid = 1;
  assert.equal(process.ppid, 1);
  process.ppid = ppid;
});
