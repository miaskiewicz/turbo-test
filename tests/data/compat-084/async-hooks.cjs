const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const hooks = require('node:async_hooks');
const immediate = () => new Promise((resolve) => setImmediate(resolve));
testCase('async_hooks emits real PROMISE init events for native await without then patches', async () => {
  const originalThen = Promise.prototype.then; const events = [];
  const hook = hooks.createHook({ init(id, type, trigger, resource) { if (type === 'PROMISE') events.push({id, trigger, resource}); } }).enable();
  try { await (async () => { await Promise.resolve(1); return 2; })(); }
  finally { hook.disable(); }
  assert.equal(Promise.prototype.then, originalThen);
  assert.ok(events.length > 0);
  assert.equal(new Set(events.map((event) => event.id)).size, events.length);
  for (const event of events) { assert.ok(Number.isInteger(event.id) && event.id > 0); assert.ok(Number.isInteger(event.trigger) && event.trigger >= 0); assert.equal(Object.prototype.toString.call(event.resource), '[object Promise]'); }
});
testCase('promise before after and resolve hooks track the executing resource', async () => {
  const events = []; let current; let trigger; let resource;
  const hook = hooks.createHook({
    init(id, type, parent) { if (type === 'PROMISE') events.push(['init', id, parent]); },
    before(id) { events.push(['before', id, hooks.executionAsyncId()]); },
    after(id) { events.push(['after', id, hooks.executionAsyncId()]); },
    promiseResolve(id) { events.push(['resolve', id]); },
  }).enable();
  try {
    await Promise.resolve().then(() => { current = hooks.executionAsyncId(); trigger = hooks.triggerAsyncId(); resource = hooks.executionAsyncResource(); });
  } finally { hook.disable(); }
  const init = events.find((event) => event[0] === 'init' && event[1] === current);
  assert.ok(init); assert.equal(init[2], trigger); assert.equal(resource instanceof Promise, true);
  const before = events.findIndex((event) => event[0] === 'before' && event[1] === current);
  const after = events.findIndex((event) => event[0] === 'after' && event[1] === current);
  assert.ok(before >= 0 && after > before);
  assert.equal(events[before][2], current); assert.equal(events[after][2], current);
  assert.ok(events.some((event) => event[0] === 'resolve' && event[1] === current));
});
testCase('promise trigger ids identify the actual parent promise', async () => {
  const records = new Map();
  const hook = hooks.createHook({ init(id, type, trigger, resource) { if (type === 'PROMISE') records.set(resource, {id, trigger}); } }).enable();
  let parent; let child;
  try { parent = Promise.resolve('p'); child = parent.then((value) => value); await child; }
  finally { hook.disable(); }
  assert.equal(records.get(child).trigger, records.get(parent).id);
});
testCase('hook enable is idempotent and disable stops observations', () => {
  let count = 0;
  const hook = hooks.createHook({ init(id, type) { if (type === 'PROMISE') count++; } });
  assert.equal(hook.enable(), hook); assert.equal(hook.enable(), hook);
  Promise.resolve(); assert.equal(count, 1);
  assert.equal(hook.disable(), hook); assert.equal(hook.disable(), hook);
  Promise.resolve(); assert.equal(count, 1);
});
testCase('AsyncResource init before after destroy carry real resource identities', async () => {
  const seen = []; const resources = new Map(); let execution; let trigger;
  const hook = hooks.createHook({
    init(id, type, parent, resource) { resources.set(id, resource); if (type === 'CompatResource') seen.push(['init', id, parent]); },
    before(id) { seen.push(['before', id]); }, after(id) { seen.push(['after', id]); }, destroy(id) { seen.push(['destroy', id]); },
  }).enable();
  let resource;
  try {
    resource = new hooks.AsyncResource('CompatResource', {triggerAsyncId: 123});
    const receiver = {};
    assert.equal(resource.runInAsyncScope(function(value) { assert.equal(this, receiver); execution = hooks.executionAsyncId(); trigger = hooks.triggerAsyncId(); assert.equal(hooks.executionAsyncResource(), resource); return value; }, receiver, 42), 42);
    resource.emitDestroy(); await immediate();
  } finally { hook.disable(); }
  assert.equal(execution, resource.asyncId()); assert.equal(trigger, 123);
  assert.equal(resource.triggerAsyncId(), 123); assert.equal(resources.get(execution), resource);
  for (const phase of ['init', 'before', 'after', 'destroy']) assert.ok(seen.some((event) => event[0] === phase && event[1] === execution));
});
testCase('timers immediates and nextTick callbacks have their own observed resources', async () => {
  const records = new Map(); const executions = [];
  const hook = hooks.createHook({ init(id, type) { records.set(id, type); } }).enable();
  try {
    await new Promise((resolve) => setTimeout(() => { executions.push([hooks.executionAsyncId(), 'Timeout']); resolve(); }, 1));
    await new Promise((resolve) => setImmediate(() => { executions.push([hooks.executionAsyncId(), 'Immediate']); resolve(); }));
    await new Promise((resolve) => process.nextTick(() => { executions.push([hooks.executionAsyncId(), 'TickObject']); resolve(); }));
  } finally { hook.disable(); }
  for (const [id, type] of executions) assert.equal(records.get(id), type);
  assert.equal(new Set(executions.map(([id]) => id)).size, 3);
});
testCase('cancelled timers emit destroy without executing their callbacks', async () => {
  const destroyed = new Set(); let timerId; let called = false; let creating = false;
  function cancelled() { called = true; }
  const hook = hooks.createHook({ init(id, type, trigger, resource) { if (type === 'Timeout' && creating) timerId = id; }, destroy(id) { destroyed.add(id); } }).enable();
  try { creating = true; const timer = setTimeout(cancelled, 60000); creating = false; clearTimeout(timer); await immediate(); }
  finally { hook.disable(); }
  assert.ok(timerId > 0); assert.equal(called, false); assert.equal(destroyed.has(timerId), true);
});
testCase('promiseResolve is not fabricated as premature promise destroy', async () => {
  const ids = new WeakMap(); const resolved = new Set(); const destroyed = new Set();
  const hook = hooks.createHook({ init(id, type, trigger, resource) { if (type === 'PROMISE') ids.set(resource, id); }, promiseResolve(id) { resolved.add(id); }, destroy(id) { destroyed.add(id); } }).enable();
  let retained;
  try { retained = Promise.resolve('retained'); await retained; await immediate(); assert.equal(resolved.has(ids.get(retained)), true); assert.equal(destroyed.has(ids.get(retained)), false); assert.equal(await retained, 'retained'); }
  finally { hook.disable(); }
});
testCase('createHook validates callback types instead of installing inert callbacks', () => {
  for (const name of ['init', 'before', 'after', 'destroy', 'promiseResolve']) assert.throws(() => hooks.createHook({[name]: 1}), {code:'ERR_ASYNC_CALLBACK'});
});
testCase('native hooks preserve AsyncLocalStorage across concurrent awaits and timers', async () => {
  const storage = new hooks.AsyncLocalStorage(); let promises = 0;
  const hook = hooks.createHook({ init(id, type) { if (type === 'PROMISE') promises++; } }).enable();
  try {
    const run = (value) => storage.run(value, async () => { await Promise.resolve(); await new Promise((resolve) => setTimeout(resolve, 1)); return storage.getStore(); });
    assert.deepEqual(await Promise.all([run('a'), run('b')]), ['a', 'b']);
    assert.ok(promises > 0); assert.equal(storage.getStore(), undefined);
  } finally { hook.disable(); }
});
