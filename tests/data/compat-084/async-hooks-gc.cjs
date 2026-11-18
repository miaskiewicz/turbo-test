const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const hooks = require('node:async_hooks');
async function collectUntil(predicate) {
  for (let attempt = 0; attempt < 40 && !predicate(); attempt++) {
    globalThis.gc();
    await new Promise((resolve) => setImmediate(resolve));
  }
}
testCase('automatic AsyncResource destruction follows collection and honors manual ownership', async () => {
  const destroyed = new Set();
  const hook = hooks.createHook({ destroy(id) { destroyed.add(id); } }).enable();
  function abandon(manual) { return new hooks.AsyncResource('GC_CONTROL', { requireManualDestroy: manual }).asyncId(); }
  try {
    const automatic = abandon(false); const manual = abandon(true);
    await collectUntil(() => destroyed.has(automatic));
    assert.equal(destroyed.has(automatic), true);
    assert.equal(destroyed.has(manual), false);
  } finally { hook.disable(); }
});
testCase('unreachable Promise destruction follows collection rather than resolution', async () => {
  const destroyed = new Set(); let target;
  const hook = hooks.createHook({ init(id, type) { if (type === 'PROMISE' && target === undefined) target = id; }, destroy(id) { destroyed.add(id); } }).enable();
  try {
    let retained = Promise.resolve('retained');
    await collectUntil(() => false);
    assert.equal(destroyed.has(target), false);
    assert.equal(await retained, 'retained'); retained = undefined;
    await collectUntil(() => destroyed.has(target));
    assert.equal(destroyed.has(target), true);
  } finally { hook.disable(); }
});
