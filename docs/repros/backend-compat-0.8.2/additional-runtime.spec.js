test('preserves per-request AsyncLocalStorage across concurrent awaits', async () => {
  const { AsyncLocalStorage } = require('node:async_hooks');
  const storage = new AsyncLocalStorage();
  const read = id => storage.run(id, async () => {
    await Promise.resolve();
    return storage.getStore();
  });
  expect(await Promise.all([read('a'), read('b')])).toEqual(['a', 'b']);
});
test('computes the SHA-256 known answer', () => {
  const { createHash } = require('node:crypto');
  expect(createHash('sha256').update('abc').digest('hex')).toBe(
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  );
});
test('writes a gzip header', () => {
  const { gzipSync } = require('node:zlib');
  expect([...gzipSync(Buffer.from('hello')).subarray(0, 2)]).toEqual([0x1f, 0x8b]);
});
test('aborts after the timeout', async () => {
  const signal = AbortSignal.timeout(1);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(signal.aborted).toBe(true);
  expect(signal.reason.name).toBe('TimeoutError');
});
test('fetches an in-memory data URL', async () => {
  const response = await fetch('data:text/plain,hello');
  expect(await response.text()).toBe('hello');
});
test('preserves a Map and detaches a transferred ArrayBuffer', () => {
  const buffer = new Uint8Array([42]).buffer;
  const clone = structuredClone(new Map([['value', buffer]]), { transfer: [buffer] });
  expect(buffer.byteLength).toBe(0);
  expect([...new Uint8Array(clone.get('value'))]).toEqual([42]);
});
test('transfers a MessagePort', () => {
  const { MessageChannel } = require('node:worker_threads');
  const { port1, port2 } = new MessageChannel();
  try {
    const moved = structuredClone(port1, { transfer: [port1] });
    moved.close();
  } finally {
    port1.close();
    port2.close();
  }
});
test('exposes Web Crypto digest', async () => {
  const hash = await globalThis.crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc'));
  expect(Buffer.from(hash).toString('hex')).toBe(
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  );
});
