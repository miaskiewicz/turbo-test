// Node passes a written Buffer to _write as the same object, and a Uint8Array as a Buffer view
// over the same memory (Stream._uint8ArrayToBuffer) — never a copy. Checked with node --test.
const test = globalThis.test || require('node:test');
const assert = require('node:assert');
const { Writable, Transform } = require('node:stream');

function collector() {
  const seen = [];
  const w = new Writable({ write(chunk, encoding, cb) { seen.push({ chunk, encoding }); cb(); } });
  return { w, seen };
}

test('a written Buffer reaches _write as the same object', () => {
  const { w, seen } = collector();
  const b = Buffer.from('ab');
  const empty = Buffer.alloc(0);
  w.write(b);
  w.write(empty);
  assert.strictEqual(seen[0].chunk, b);
  assert.strictEqual(seen[0].encoding, 'buffer');
  assert.strictEqual(seen[1].chunk, empty);
});

test('a written Uint8Array reaches _write as a Buffer view over the same memory', () => {
  const { w, seen } = collector();
  const backing = new Uint8Array([9, 1, 2, 3, 9]);
  const u = backing.subarray(1, 4);
  w.write(u);
  const got = seen[0].chunk;
  assert.ok(Buffer.isBuffer(got));
  assert.strictEqual(got.buffer, backing.buffer);
  assert.strictEqual(got.byteOffset, 1);
  assert.strictEqual(got.length, 3);
  backing[2] = 7;
  assert.strictEqual(got[1], 7);
});

test('a buffered Buffer keeps its identity through _transform', async () => {
  const b1 = Buffer.from('x'), b2 = Buffer.from('y');
  const seen = [];
  const t = new Transform({ transform(chunk, e, cb) { seen.push(chunk); setImmediate(cb); } });
  t.resume();
  t.write(b1);
  t.write(b2);
  await new Promise((resolve) => t.end(resolve));
  assert.strictEqual(seen[0], b1);
  assert.strictEqual(seen[1], b2);
});
