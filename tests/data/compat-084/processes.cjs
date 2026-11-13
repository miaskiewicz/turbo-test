const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const node = process.execPath;

testCase('spawnSync executes the host Node executable and returns actual stdout', () => {
  assert.equal(typeof node, 'string');
  assert.equal(path.isAbsolute(node), true);
  const result = cp.spawnSync(node, ['-e', 'process.stdout.write("ok")'], { encoding: 'utf8' });
  assert.equal(result.status, 0);
  assert.equal(result.signal, null);
  assert.equal(result.stdout, 'ok');
  assert.equal(result.stderr, '');
  assert.equal(result.error, undefined);
  assert.equal(result.output[1], result.stdout);
  assert.equal(result.output[2], result.stderr);
  assert.equal(result.pid > 0, true);
});
testCase('spawnSync preserves binary output stderr and a real nonzero status', () => {
  const result = cp.spawnSync(node, ['-e', 'process.stdout.write(Buffer.from([0,255,1]));process.stderr.write("bad");process.exitCode=7']);
  assert.equal(result.status, 7);
  assert.equal(Buffer.isBuffer(result.stdout), true);
  assert.deepEqual(Array.from(result.stdout), [0, 255, 1]);
  assert.equal(result.stderr.toString(), 'bad');
  assert.equal(cp.spawnSync(node, ['-e', 'process.stdout.write(Buffer.from([0,255]))'], { encoding: 'hex' }).stdout, '00ff');
  assert.equal(cp.spawnSync(node, ['-e', 'process.stdin.on("data",b=>process.stdout.write(b))'], { input: '61zz', encoding: 'hex' }).stdout, '61');
});
testCase('spawnSync forwards arguments without shell interpolation and accepts input byte views', () => {
  const data = new Uint8Array([0, 65, 255, 66, 0]);
  const result = cp.spawnSync(node, ['-e', 'process.stdin.on("data",b=>process.stdout.write(b));process.stderr.write(JSON.stringify(process.argv.slice(1)))', 'a b', '$(not-a-command)', '"quoted"'], { input: data.subarray(1, 4) });
  assert.equal(result.status, 0);
  assert.deepEqual(Array.from(result.stdout), [65, 255, 66]);
  assert.deepEqual(JSON.parse(result.stderr.toString()), ['a b', '$(not-a-command)', '"quoted"']);
});
testCase('spawnSync applies cwd and an explicit environment', () => {
  // The harness gives each batch its own canonical fixture directory.
  const directory = path.join(__dirname, 'process-cwd-fixture');
  fs.mkdirSync(directory, { recursive: true });
  try {
    const result = cp.spawnSync(node, ['-e', 'process.stdout.write(JSON.stringify([process.cwd(),process.env.TURBO_PROCESS_VALUE,process.env.TURBO_PROCESS_MISSING]))'], { cwd: directory, env: { TURBO_PROCESS_VALUE: 'actual' }, encoding: 'utf8' });
    assert.equal(result.status, 0);
    assert.deepEqual(JSON.parse(result.stdout), [directory, 'actual', null]);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
  const previous = process.env.TURBO_PROCESS_MUTATED;
  process.env.TURBO_PROCESS_MUTATED = 'new-value';
  try {
    assert.equal(cp.spawnSync(node, ['-e', 'process.stdout.write(process.env.TURBO_PROCESS_MUTATED)'], { encoding: 'utf8' }).stdout, 'new-value');
  } finally {
    if (previous === undefined) delete process.env.TURBO_PROCESS_MUTATED;
    else process.env.TURBO_PROCESS_MUTATED = previous;
  }
});
testCase('spawnSync concurrently drains stdout stderr and stdin without truncation', () => {
  const input = 'x'.repeat(131072);
  const result = cp.spawnSync(node, ['-e', 'process.stderr.write("e".repeat(131072));process.stdin.on("data",b=>process.stdout.write(b))'], { input, encoding: 'utf8', maxBuffer: 1048576 });
  assert.equal(result.status, 0);
  assert.equal(result.stdout, input);
  assert.equal(result.stderr.length, 131072);
  const overflow = cp.spawnSync(node, ['-e', 'process.stdout.write("x".repeat(131072))'], { maxBuffer: 1024 });
  assert.equal(overflow.error.code, 'ENOBUFS');
  assert.equal(Buffer.isBuffer(overflow.stdout), true);
});
testCase('spawnSync reports missing executables and invalid input truthfully', () => {
  const result = cp.spawnSync(path.join(os.tmpdir(), 'turbo-no-such-executable-084'), [], { encoding: 'utf8' });
  assert.equal(result.status, null);
  assert.equal(result.error.code, 'ENOENT');
  assert.equal(result.error.path.endsWith('turbo-no-such-executable-084'), true);
  assert.throws(() => cp.spawnSync(42), TypeError);
  assert.throws(() => cp.spawnSync(node, [], { timeout: -1 }), RangeError);
  assert.throws(() => cp.spawnSync(node, [], { input: {} }), TypeError);
  const unread = cp.spawnSync(node, ['-e', 'process.exit(0)'], { input: new Uint8Array(2097152) });
  assert.equal(unread.status, 0);
  assert.equal(unread.error.code, 'EPIPE');
});
testCase('spawnSync enforces timeout and remains usable after killing the child', () => {
  const result = cp.spawnSync(node, ['-e', 'setInterval(()=>{},1000)'], { timeout: 30, encoding: 'utf8' });
  assert.equal(result.status, null);
  assert.equal(result.error.code, 'ETIMEDOUT');
  assert.equal(typeof result.signal, 'string');
  assert.equal(cp.spawnSync(node, ['-e', 'process.stdout.write("after")'], { encoding: 'utf8' }).stdout, 'after');
});
testCase('execFileSync and execSync return output and throw rich errors on failure', () => {
  assert.equal(cp.execFileSync(node, ['-e', 'process.stdout.write("file")'], { encoding: 'utf8' }), 'file');
  assert.equal(cp.execSync('printf shell-output', { encoding: 'utf8' }), 'shell-output');
  assert.throws(() => cp.execFileSync(node, ['-e', 'process.stdout.write("out");process.stderr.write("err");process.exit(9)'], { encoding: 'utf8', stdio: 'pipe' }), (error) => error.status === 9 && error.stdout === 'out' && error.stderr === 'err');
});
