const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { MessageChannel, MessagePort } = require('node:worker_threads');
const { setTimeout: realTimeout } = require('node:timers');
const wait = () => new Promise((resolve) => realTimeout(resolve, 10));
const receive = (port) => new Promise((resolve) => port.once('message', resolve));
const close = (...ports) => ports.forEach((port) => port.close());

testCase('MessagePort transfers to a real new handle and retains its peer', async () => {
  const { port1, port2 } = new MessageChannel();
  port1.ref(); assert.equal(port1.hasRef(), true);
  port1.unref(); assert.equal(port1.hasRef(), false);
  const moved = structuredClone(port1, { transfer: [port1] });
  assert.equal(port1.hasRef(), false);
  try {
    assert.equal(moved instanceof MessagePort, true);
    assert.notEqual(moved, port1);
    const message = receive(moved); port2.postMessage('hello');
    assert.equal(await message, 'hello');
    const reply = receive(port2); moved.postMessage('reply');
    assert.equal(await reply, 'reply');
  } finally { close(port1, port2, moved); }
  const altered = new MessageChannel();
  Object.setPrototypeOf(altered.port1, null);
  const restored = structuredClone(altered.port1, { transfer: [altered.port1] });
  try {
    assert.equal(restored instanceof MessagePort, true);
    const received = receive(restored); altered.port2.postMessage('prototype');
    assert.equal(await received, 'prototype');
  } finally { MessagePort.prototype.close.call(altered.port1); close(altered.port2, restored); }
});
testCase('queued messages survive transfer in order and old listeners do not receive them', async () => {
  const { port1, port2 } = new MessageChannel();
  let originalCalls = 0;
  port1.on('message', () => originalCalls++);
  port2.postMessage('one'); port2.postMessage('two');
  const moved = structuredClone(port1, { transfer: [port1] });
  try {
    const seen = [];
    await new Promise((resolve) => moved.on('message', (value) => { seen.push(value); if (seen.length === 2) resolve(); }));
    assert.deepEqual(seen, ['one', 'two']);
    assert.equal(originalCalls, 0);
    port1.close();
    const next = receive(moved); port2.postMessage('three');
    assert.equal(await next, 'three');
  } finally { close(port1, port2, moved); }
});
testCase('native structured clone preserves nested port aliases cycles and ArrayBuffer transfer', async () => {
  const { port1, port2 } = new MessageChannel();
  const memory = new Uint8Array([1, 2, 3]).buffer;
  const source = { port: port1, map: new Map([['port', port1]]), memory }; source.self = source;
  const copy = structuredClone(source, { transfer: [port1, memory] });
  try {
    assert.equal(copy.self, copy);
    assert.equal(copy.map.get('port'), copy.port);
    assert.equal(memory.byteLength, 0);
    assert.deepEqual(Array.from(new Uint8Array(copy.memory)), [1, 2, 3]);
    const message = receive(copy.port); port2.postMessage(42); assert.equal(await message, 42);
  } finally { close(port1, port2, copy.port); }
});
testCase('failed clone does not detach transfer-list ports or ArrayBuffers', async () => {
  const { port1, port2 } = new MessageChannel();
  const memory = new Uint8Array([7]).buffer;
  try {
    assert.throws(() => structuredClone({ port1, bad() {} }, { transfer: [port1, memory] }), { name: 'DataCloneError' });
    assert.equal(memory.byteLength, 1);
    const message = receive(port1); port2.postMessage('retained'); assert.equal(await message, 'retained');
    assert.throws(() => structuredClone(port1, { transfer: [port1, port1] }), { name: 'DataCloneError' });
    const again = receive(port1); port2.postMessage('still live'); assert.equal(await again, 'still live');
    assert.throws(() => structuredClone(port1));
  } finally { close(port1, port2); }
});
testCase('a detached port cannot transfer again and posting through it does not reach its peer', async () => {
  const { port1, port2 } = new MessageChannel();
  const moved = structuredClone(port1, { transfer: [port1] });
  try {
    assert.throws(() => structuredClone(port1, { transfer: [port1] }), { name: 'DataCloneError' });
    const seen = []; port2.on('message', (value) => seen.push(value));
    port1.postMessage('detached'); moved.postMessage('live');
    await wait(); assert.deepEqual(seen, ['live']);
  } finally { close(port1, port2, moved); }
});
testCase('postMessage clones snapshots and transfers ArrayBuffer bytes', async () => {
  const { port1, port2 } = new MessageChannel();
  try {
    const value = { n: 1 }; const first = receive(port2); port1.postMessage(value); value.n = 2;
    assert.deepEqual(await first, { n: 1 });
    const memory = new Uint8Array([0, 255]).buffer; const second = receive(port2);
    port1.postMessage({ memory }, [memory]); assert.equal(memory.byteLength, 0);
    assert.deepEqual(Array.from(new Uint8Array((await second).memory)), [0, 255]);
  } finally { close(port1, port2); }
});
testCase('a port can be transferred through another channel and then transferred again', async () => {
  const outer = new MessageChannel(); const inner = new MessageChannel(); let moved; let movedAgain;
  try {
    const received = receive(outer.port2); outer.port1.postMessage({ port: inner.port1 }, { transfer: (function* () { yield inner.port1; })() });
    moved = (await received).port;
    const message = receive(moved); inner.port2.postMessage('forwarded'); assert.equal(await message, 'forwarded');
    inner.port2.postMessage('queued again');
    movedAgain = structuredClone(moved, { transfer: [moved] });
    assert.equal(await receive(movedAgain), 'queued again');
  } finally { close(outer.port1, outer.port2, inner.port1, inner.port2); if (moved) moved.close(); if (movedAgain) movedAgain.close(); }
});
testCase('addEventListener and onmessage receive MessageEvent data asynchronously', async () => {
  const { port1, port2 } = new MessageChannel();
  try {
    const order = []; let calls = 0;
    const listener = (event) => { calls++; assert.equal(event.target, port2); assert.equal(event.data, 'event'); order.push('message'); };
    port2.addEventListener('message', listener); port2.start();
    const received = new Promise((resolve) => { port2.onmessage = (event) => resolve(event.data); });
    port1.postMessage('event'); order.push('sync');
    assert.equal(await received, 'event'); assert.deepEqual(order, ['sync', 'message']);
    port2.removeEventListener('message', listener);
    const again = new Promise((resolve) => { port2.onmessage = (event) => resolve(event.data); });
    port1.postMessage('other'); assert.equal(await again, 'other'); assert.equal(calls, 1);
  } finally { close(port1, port2); }
});

testCase('MessagePort callbacks run in the receiving resource async context', async () => {
  const { AsyncLocalStorage } = require('node:async_hooks');
  const storage = new AsyncLocalStorage();
  for (const move of [false, true]) {
    const channel = storage.run('created', () => new MessageChannel());
    let receiver = channel.port2;
    if (move) receiver = storage.run('moved', () => structuredClone(receiver, { transfer: [receiver] }));
    try {
      const context = new Promise((resolve) => receiver.once('message', () => resolve(storage.getStore())));
      storage.run('sender', () => channel.port1.postMessage('context'));
      assert.equal(await context, move ? 'moved' : 'created');
      assert.equal(storage.getStore(), undefined);
    } finally { close(channel.port1, channel.port2, receiver); }
  }
});
