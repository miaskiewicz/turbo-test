const assert = require('node:assert/strict');
const { AsyncLocalStorage, AsyncResource } = require('node:async_hooks');
const nativeGet = globalThis.__ttAsyncFrameGet;
assert.equal(typeof nativeGet, 'function');
let nativeReads = 0;
globalThis.__ttAsyncFrameGet = function () { nativeReads++; return nativeGet(); };
const storage = new AsyncLocalStorage();
const beforeResource = new AsyncResource('before-first-frame');
const beforeBound = AsyncLocalStorage.bind(() => storage.getStore());
const beforeSnapshot = AsyncLocalStorage.snapshot();
const wait = () => new Promise((resolve) => setTimeout(resolve, 1));

test('unused async context requires no native frame reads for DOM work, timers and nextTicks', async () => {
  const tasks = [];
  for (let i = 0; i < 16; i++) {
    tasks.push(new Promise((resolve) => setTimeout(resolve, 1)));
    tasks.push(new Promise((resolve) => setImmediate(resolve)));
    tasks.push(new Promise((resolve) => process.nextTick(resolve)));
  }
  await Promise.all(tasks);
  if (globalThis.__expectLazyDom) {
    const element = document.createElement('div'); element.textContent = 'unchanged DOM work';
    document.body.appendChild(element); assert.equal(element.textContent, 'unchanged DOM work');
    element.remove();
  }
  assert.equal(storage.getStore(), undefined);
  assert.equal(nativeReads, 0);
});
test('first active storage frame retains native await, timer and nextTick propagation', async () => {
  const values = await storage.run('active', async () => {
    await Promise.resolve();
    const timer = new Promise((resolve) => setTimeout(() => resolve(storage.getStore()), 1));
    const tick = new Promise((resolve) => process.nextTick(() => resolve(storage.getStore())));
    return [storage.getStore(), await timer, await tick];
  });
  assert.deepEqual(values, ['active', 'active', 'active']);
  assert.equal(storage.getStore(), undefined); assert.ok(nativeReads > 0);
});
test('resources, bind and snapshot captured before activation clear a later callers store', async () => {
  await storage.run('caller', async () => {
    assert.equal(beforeBound(), undefined);
    assert.equal(beforeResource.runInAsyncScope(() => storage.getStore()), undefined);
    const value = await beforeSnapshot(async () => { await wait(); return storage.getStore(); });
    assert.equal(value, undefined); assert.equal(storage.getStore(), 'caller');
  });
  assert.equal(storage.getStore(), undefined);
});
test('enterWith, disable and reactivation preserve their original context semantics', async () => {
  await storage.run('outer', async () => {
    storage.enterWith('entered'); await wait(); assert.equal(storage.getStore(), 'entered');
    storage.disable(); assert.equal(storage.getStore(), undefined);
  });
  assert.equal(await storage.run('new', async () => { await wait(); return storage.getStore(); }), 'new');
  assert.equal(storage.getStore(), undefined);
});
