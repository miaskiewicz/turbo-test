const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const bare = () => Object.create(EventEmitter.prototype);
test('constructor-free reads are empty and leave receiver untouched', () => {
  const e = bare(), keys = Reflect.ownKeys(e);
  assert.equal(e.emit('init'), false);
  assert.equal(e.listenerCount('init'), 0);
  assert.deepEqual(e.listeners('init'), []);
  assert.deepEqual(Reflect.ownKeys(e), keys);
  assert.equal(e.off('init', () => {}), e);
  assert.equal(e.removeAllListeners('init'), e);
  assert.equal(e.removeAllListeners(), e);
});
test('constructor-free registration isolates inherited and sibling state', () => {
  const parent = bare(), child = Object.create(parent), sibling = Object.create(parent);
  let inherited = 0, own = 0;
  parent.on('init', () => inherited++);
  child.on('init', () => own++);
  child.emit('init');
  assert.equal(inherited, 1); assert.equal(own, 1);
  sibling.emit('init');
  assert.equal(inherited, 2); assert.equal(own, 2);
  const isolated = bare();
  assert.equal(isolated.listenerCount('init'), 0);
});
test('borrowed prototype methods operate on a plain object', () => {
  const e = {}, seen = [], p = EventEmitter.prototype;
  const listener = value => seen.push(value);
  assert.equal(p.on.call(e, 'x', listener), e);
  assert.equal(p.emit.call(e, 'x', 3), true);
  assert.deepEqual(seen, [3]);
  assert.equal(p.listenerCount.call(e, 'x'), 1);
  assert.equal(p.off.call(e, 'x', listener), e);
  assert.equal(p.emit.call(e, 'x'), false);
});
test('constructor-free once removal and prepend preserve ordering', () => {
  const e = bare(), seen = [];
  const once = () => seen.push('once');
  e.once('x', once); e.prependListener('x', () => seen.push('first'));
  assert.equal(e.listeners('x')[1], once);
  e.emit('x'); e.emit('x');
  assert.deepEqual(seen, ['first', 'once', 'first']);
  e.once('y', once); e.removeListener('y', once);
  assert.equal(e.emit('y'), false);
});
test('constructor-free unhandled errors keep real Node semantics', () => {
  const e = bare(), error = new Error('sentinel');
  assert.throws(() => e.emit('error', error), value => value === error);
  assert.throws(() => e.emit('error', 'context'), { code: 'ERR_UNHANDLED_ERROR', context: 'context' });
  let seen;
  e.on('error', value => seen = value);
  assert.equal(e.emit('error', error), true); assert.equal(seen, error);
  e.removeAllListeners();
  assert.throws(() => e.emit('error', error), value => value === error);
});
test('enumerable prototype mixins retain callable event methods', () => {
  function Model() {}
  for (const name in EventEmitter.prototype) Model[name] = EventEmitter.prototype[name];
  const seen = [];
  assert.equal(typeof Model.on, 'function'); assert.equal(typeof Model.emit, 'function');
  Model.once('init', value => seen.push(value)); Model.emit('init', 7); Model.emit('init', 8);
  assert.deepEqual(seen, [7]);
  assert.equal(Model.listenerCount('init'), 0);
});
