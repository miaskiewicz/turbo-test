const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
// readable-stream v4 (archiver's stream library) overrides on / removeListener, calls the
// EventEmitter originals from them and aliases addListener / off to its overrides.
function Legacy() { EventEmitter.call(this); this.added = 0; this.removed = 0; }
Object.setPrototypeOf(Legacy.prototype, EventEmitter.prototype);
Legacy.prototype.on = function(event, fn) { this.added++; return EventEmitter.prototype.on.call(this, event, fn); };
Legacy.prototype.addListener = Legacy.prototype.on;
Legacy.prototype.removeListener = function(event, fn) { this.removed++; return EventEmitter.prototype.removeListener.call(this, event, fn); };
Legacy.prototype.off = Legacy.prototype.removeListener;

test('addListener and off are the same functions as on and removeListener', () => {
  const p = EventEmitter.prototype;
  assert.equal(p.addListener, p.on);
  assert.equal(p.off, p.removeListener);
});
test('aliased overrides calling the originals do not recurse', () => {
  const emitter = new Legacy();
  const fn = () => {};
  emitter.addListener('x', fn);
  assert.equal(emitter.listenerCount('x'), 1);
  emitter.off('x', fn);
  assert.equal(emitter.listenerCount('x'), 0);
  assert.deepEqual([emitter.added, emitter.removed], [1, 1]);
});
test('once registers through on and unregisters through removeListener', () => {
  const emitter = new Legacy();
  let hits = 0;
  emitter.once('end', () => hits++);
  assert.equal(emitter.emit('end'), true);
  assert.equal(emitter.emit('end'), false);
  assert.equal(hits, 1);
  assert.deepEqual([emitter.added, emitter.removed], [1, 1]);
  const listener = () => hits++;
  emitter.once('end', listener);
  emitter.removeListener('end', listener);
  assert.equal(emitter.emit('end'), false);
  assert.deepEqual([hits, emitter.added, emitter.removed], [1, 2, 2]);
});
