const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { AsyncLocalStorage, AsyncResource } = require('node:async_hooks');
const wait = () => new Promise((resolve) => setTimeout(resolve, 1));

testCase('AsyncLocalStorage preserves concurrent stores across native await', async () => {
  const storage = new AsyncLocalStorage();
  const read = (id) => storage.run(id, async () => { await Promise.resolve(); await wait(); return storage.getStore(); });
  assert.deepEqual(await Promise.all([read('a'), read('b')]), ['a', 'b']);
  assert.equal(storage.getStore(), undefined);
});
testCase('AsyncLocalStorage restores nested scopes and thrown or rejected callbacks', async () => {
  const storage = new AsyncLocalStorage();
  const failure = new Error('scope failure');
  await storage.run('outer', async () => {
    assert.throws(() => storage.run('inner', () => { assert.equal(storage.getStore(), 'inner'); throw failure; }), (e) => e === failure);
    assert.equal(storage.getStore(), 'outer');
    await assert.rejects(storage.run('rejected', async () => { await Promise.resolve(); assert.equal(storage.getStore(), 'rejected'); throw failure; }), (e) => e === failure);
    assert.equal(storage.getStore(), 'outer');
  });
  assert.equal(storage.getStore(), undefined);
});
testCase('AsyncLocalStorage captures attachment context for an existing promise', async () => {
  const storage = new AsyncLocalStorage();
  const existing = Promise.resolve('ready');
  const seen = await storage.run('attached', async () => {
    const chained = existing.then(() => storage.getStore());
    await existing;
    return [storage.getStore(), await chained];
  });
  assert.deepEqual(seen, ['attached', 'attached']);
});
testCase('AsyncLocalStorage instances are independent and preserve store identity', async () => {
  const first = new AsyncLocalStorage(); const second = new AsyncLocalStorage();
  const object = { value: 42 };
  await first.run(object, async () => second.run(false, async () => {
    await Promise.resolve();
    assert.equal(first.getStore(), object); assert.equal(second.getStore(), false);
  }));
  assert.equal(first.getStore(), undefined); assert.equal(second.getStore(), undefined);
  first.run('first', () => second.enterWith('independent-change'));
  assert.equal(first.getStore(), undefined);
  assert.equal(second.getStore(), 'independent-change');
  second.disable();
});
testCase('AsyncLocalStorage exit suppresses only its own store across await', async () => {
  const first = new AsyncLocalStorage(); const second = new AsyncLocalStorage();
  await first.run('first', async () => second.run('second', async () => {
    await first.exit(async () => { await wait(); assert.equal(first.getStore(), undefined); assert.equal(second.getStore(), 'second'); });
    assert.equal(first.getStore(), 'first'); assert.equal(second.getStore(), 'second');
  }));
});
testCase('AsyncLocalStorage disable clears the current scope and run re-enables it', async () => {
  const storage = new AsyncLocalStorage();
  storage.run('old', () => { storage.disable(); assert.equal(storage.getStore(), undefined); });
  assert.equal(storage.getStore(), undefined);
  assert.equal(await storage.run('new', async () => { await wait(); return storage.getStore(); }), 'new');
  assert.equal(storage.getStore(), undefined);
});
testCase('AsyncLocalStorage enterWith propagates to descendants without leaking after run', async () => {
  const storage = new AsyncLocalStorage();
  await storage.run('outer', async () => {
    storage.enterWith('entered');
    await Promise.resolve();
    assert.equal(storage.getStore(), 'entered');
  });
  assert.equal(storage.getStore(), undefined);
});
testCase('AsyncLocalStorage captures timer nextTick and queueMicrotask contexts', async () => {
  const storage = new AsyncLocalStorage();
  const create = (value) => storage.run(value, () => Promise.all([
    new Promise((resolve) => setTimeout(() => resolve(storage.getStore()), 1)),
    new Promise((resolve) => setImmediate(() => resolve(storage.getStore()))),
    new Promise((resolve) => process.nextTick(() => resolve(storage.getStore()))),
    new Promise((resolve) => queueMicrotask(() => resolve(storage.getStore()))),
  ]));
  assert.deepEqual(await Promise.all([create('a'), create('b')]), [['a', 'a', 'a', 'a'], ['b', 'b', 'b', 'b']]);
});
testCase('AsyncResource restores construction context and callback receiver', async () => {
  const storage = new AsyncLocalStorage();
  const resource = storage.run('created', () => new AsyncResource('compat-resource'));
  const receiver = { marker: 1 };
  const result = storage.run('caller', () => resource.runInAsyncScope(function(arg) {
    assert.equal(this, receiver); assert.equal(arg, 42); return storage.getStore();
  }, receiver, 42));
  assert.equal(result, 'created'); assert.equal(storage.getStore(), undefined);
  resource.emitDestroy();
});
testCase('AsyncLocalStorage bind and snapshot preserve captured scope', async () => {
  const storage = new AsyncLocalStorage();
  const bound = storage.run('bound', () => AsyncLocalStorage.bind((value) => [storage.getStore(), value]));
  const snapshot = storage.run('snapshot', () => AsyncLocalStorage.snapshot());
  assert.deepEqual(bound(42), ['bound', 42]);
  assert.equal(await snapshot(async () => { await wait(); return storage.getStore(); }), 'snapshot');
  assert.equal(storage.getStore(), undefined);
});
