const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { spawn, fork } = require('node:child_process');
const path = require('node:path');
const clean = () => ({ ASYNC_PROCESS_MARKER: 'clean' });
const closed = (child) => new Promise((resolve, reject) => {
  child.once('error', reject);
  child.once('close', (code, signal) => resolve({ code, signal }));
});
const once = (emitter, name) => new Promise((resolve) => emitter.once(name, (...args) => resolve(args)));

testCase('spawn returns before stdin exchange and delivers real bytes and event order', async () => {
  const child = spawn(process.execPath, ['-e', `
    process.stdin.on('data', chunk => process.stdout.write(chunk));
    process.stdin.on('end', () => { process.stderr.write(process.env.ASYNC_PROCESS_MARKER); process.exitCode = 7; });
  `], { env: clean(), cwd: __dirname });
  const events = ['returned']; const output = []; const errors = [];
  child.on('spawn', () => events.push('spawn'));
  child.stdout.on('data', (chunk) => { assert.equal(Buffer.isBuffer(chunk), true); output.push(chunk); events.push('data'); });
  child.stderr.on('data', (chunk) => errors.push(chunk));
  child.on('exit', () => events.push('exit'));
  child.on('close', () => events.push('close'));
  const completed = closed(child);
  assert.equal(typeof child.pid, 'number');
  assert.equal(child.spawnfile, process.execPath);
  assert.equal(child.stdio[0], child.stdin);
  child.stdin.end(Buffer.from([0, 255, 65]));
  assert.deepEqual(await completed, { code: 7, signal: null });
  assert.deepEqual([...Buffer.concat(output)], [0, 255, 65]);
  assert.equal(Buffer.concat(errors).toString(), 'clean');
  assert.equal(events[0], 'returned'); assert.equal(events[1], 'spawn');
  assert.equal(events.indexOf('exit') < events.indexOf('close'), true);
  assert.equal(child.exitCode, 7); assert.equal(child.signalCode, null);
});

testCase('spawn reports missing executables asynchronously without a fake process', async () => {
  const child = spawn('/turbo-test-missing-child-executable-371', [], { env: clean() });
  const events = []; let failure;
  child.on('spawn', () => events.push('spawn'));
  child.on('error', (error) => { failure = error; events.push('error'); });
  child.on('exit', () => events.push('exit'));
  const completed = once(child, 'close');
  events.push('returned');
  await completed;
  assert.equal(child.pid, undefined);
  assert.equal(failure.code, 'ENOENT');
  assert.equal(failure.path, '/turbo-test-missing-child-executable-371');
  assert.deepEqual(events, ['returned', 'error']);
});

testCase('kill signals the owned native process and close follows exit', async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { env: clean() });
  const completed = closed(child); await once(child, 'spawn');
  assert.equal(child.killed, false);
  assert.equal(child.kill('SIGTERM'), true);
  assert.equal(child.killed, true);
  assert.deepEqual(await completed, { code: null, signal: 'SIGTERM' });
  assert.equal(child.kill(), false);
});

testCase('spawn timeout terminates the real child with the selected signal', async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { env: clean(), timeout: 40, killSignal: 'SIGTERM' });
  assert.deepEqual(await closed(child), { code: null, signal: 'SIGTERM' });
  assert.equal(child.killed, true);
});

testCase('spawn stdout backpressure preserves bytes across pause and resume', async () => {
  const child = spawn(process.execPath, ['-e', 'process.stdout.write(Buffer.alloc(131072, 123))'], { env: clean() });
  const chunks = []; const completed = closed(child);
  child.stdout.pause();
  child.stdout.on('data', (chunk) => chunks.push(chunk));
  child.stdout.pause(); setTimeout(() => child.stdout.resume(), 5);
  assert.deepEqual(await completed, { code: 0, signal: null });
  const bytes = Buffer.concat(chunks);
  assert.equal(bytes.length, 131072);
  assert.equal(bytes.every((byte) => byte === 123), true);
});

testCase('fork exchanges JSON through a real Node IPC channel then disconnects cleanly', async () => {
  const child = fork(path.join(__dirname, 'async-fork-child.cjs'), [], { env: clean(), silent: true });
  const completed = closed(child); const message = once(child, 'message'); let sends = 0;
  assert.equal(child.connected, true);
  const sent = new Promise((resolve, reject) => child.send({ value: { bytes: [0, 255], text: 'héllo' } }, (error) => { sends++; error ? reject(error) : resolve(); }));
  const [reply] = await message; await sent;
  assert.equal(reply.pid, child.pid); assert.notEqual(reply.pid, process.pid);
  assert.equal(reply.marker, 'clean');
  assert.deepEqual(reply.value, { bytes: [0, 255], text: 'héllo' });
  assert.equal(sends, 1);
  const disconnect = once(child, 'disconnect'); child.send({ kind: 'finish' });
  await disconnect; assert.equal(child.connected, false);
  assert.deepEqual(await completed, { code: 0, signal: null });
});

testCase('parent disconnect closes the real IPC channel and later send fails', async () => {
  const child = fork(path.join(__dirname, 'async-fork-child.cjs'), [], { env: clean(), silent: true });
  const completed = once(child, 'exit'); await once(child, 'spawn');
  const ready = once(child, 'message'); child.send({ value: 'ready' }); await ready;
  const disconnected = once(child, 'disconnect'); child.disconnect();
  assert.equal(child.connected, false); await disconnected;
  assert.equal(child.connected, false);
  const failure = await new Promise((resolve) => child.send({ value: 1 }, (error) => resolve(error)));
  assert.equal(failure.code, 'ERR_IPC_CHANNEL_CLOSED');
  assert.deepEqual(await completed, [0, null]);
});

testCase('asynchronous process entry points validate arguments before creating children', () => {
  assert.throws(() => spawn(1), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => spawn(process.execPath, 'not-an-array'), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => fork(1), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => spawn(process.execPath, [], { timeout: -1 }), { code: 'ERR_OUT_OF_RANGE' });
});

testCase('real child I/O is not starved by continuously queued immediates', async () => {
  const child = spawn(process.execPath, ['-e', 'process.stdout.write("ready")'], { env: clean() });
  const completed = closed(child); const chunks = []; let done = false;
  child.stdout.on('data', (chunk) => { chunks.push(chunk); done = true; });
  const busy = () => { if (!done) setImmediate(busy); };
  setImmediate(busy);
  try {
    assert.deepEqual(await completed, { code: 0, signal: null });
    assert.equal(Buffer.concat(chunks).toString(), 'ready');
  } finally { done = true; }
});
