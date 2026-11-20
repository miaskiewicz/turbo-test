const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const events = require('node:events');
const { EventEmitter } = events;
test('EventEmitter is a callable function constructor', () => {
  assert.equal(typeof EventEmitter, 'function');
  assert.equal(events, EventEmitter);
  assert.equal(EventEmitter.prototype.constructor, EventEmitter);
  const target = {};
  assert.equal(EventEmitter.call(target, { captureRejections: false }), undefined);
  assert.throws(() => EventEmitter(), TypeError);
});
test('legacy function subclasses initialise through EventEmitter.call', () => {
  function Legacy(opts) { EventEmitter.call(this, opts); this.opts = opts; }
  Object.setPrototypeOf(Legacy.prototype, EventEmitter.prototype);
  Object.setPrototypeOf(Legacy, EventEmitter);
  const a = new Legacy({ id: 1 }), b = new Legacy({ id: 2 }), seen = [];
  a.on('x', (v) => seen.push(['a', v]));
  b.once('x', (v) => seen.push(['b', v]));
  assert.equal(a.emit('x', 1), true);
  assert.equal(b.emit('x', 2), true);
  assert.equal(b.emit('x', 3), false);
  assert.deepEqual(seen, [['a', 1], ['b', 2]]);
  assert.ok(a instanceof EventEmitter);
  assert.equal(a.opts.id, 1);
  assert.equal(a.listenerCount('x'), 1);
  assert.equal(b.listenerCount('x'), 0);
});
test('class subclasses and called prototypes keep separate listener state', () => {
  class Modern extends EventEmitter { constructor() { super(); this.kind = 'modern'; } }
  const m = new Modern();
  let hits = 0;
  m.on('y', () => hits++);
  m.emit('y');
  assert.equal(hits, 1);
  assert.equal(m.kind, 'modern');
  const proto = Object.create(EventEmitter.prototype);
  EventEmitter.call(proto);
  proto.on('z', () => hits++);
  const child = Object.create(proto);
  EventEmitter.call(child);
  assert.equal(child.emit('z'), false);
  assert.equal(proto.emit('z'), true);
  assert.equal(hits, 2);
});
