const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
// A TZDate-like subclass (@date-fns/tz's TZDateMini pattern): `super()` with no arguments, the
// time zone as the last string argument, the instant set with setTime, wall-clock fields read
// from an internal Date shifted by a fixed offset.
const OFFSETS = { UTC: 0, 'Asia/Tokyo': 540, 'America/Bogota': -300 };
class ZonedDate extends Date {
  constructor(...args) {
    super();
    if (args.length > 1 && typeof args[args.length - 1] === 'string') this.timeZone = args.pop();
    this.internal = new Date();
    if (!args.length) this.setTime(Date.now());
    else if (typeof args[0] === 'number' && args.length === 1) this.setTime(args[0]);
    else if (typeof args[0] === 'string') this.setTime(+new Date(args[0]));
    else if (args[0] instanceof Date) this.setTime(+args[0]);
    else this.setTime(Date.UTC(...args) - this.offset() * 60000);
  }
  static tz(timeZone, ...args) { return args.length ? new ZonedDate(...args, timeZone) : new ZonedDate(Date.now(), timeZone); }
  offset() { return OFFSETS[this.timeZone || 'UTC']; }
  setTime(t) { const r = Date.prototype.setTime.call(this, t); if (this.internal) this.internal.setTime(t + this.offset() * 60000); return r; }
  getHours() { return this.internal.getUTCHours(); }
  getDate() { return this.internal.getUTCDate(); }
  withTimeZone(timeZone) { return new ZonedDate(+this, timeZone); }
  [Symbol.for('constructDateFrom')](date) { return new ZonedDate(+new Date(date), this.timeZone); }
}
testCase('a Date subclass keeps new.target: prototype, methods and fields', () => {
  const d = new ZonedDate(2026, 0, 1, 'Asia/Tokyo');
  assert.equal(d instanceof ZonedDate, true);
  assert.equal(d instanceof Date, true);
  assert.equal(Object.getPrototypeOf(d), ZonedDate.prototype);
  assert.equal(Object.prototype.toString.call(d), '[object Date]');
  assert.equal(d.timeZone, 'Asia/Tokyo');
  assert.equal(d.toISOString(), '2025-12-31T15:00:00.000Z');
  assert.equal(d.getDate(), 1);
  assert.equal(d.getHours(), 0);
  const bogota = d.withTimeZone('America/Bogota');
  assert.equal(bogota instanceof ZonedDate, true);
  assert.equal(+bogota, +d);
  assert.equal(bogota.getDate(), 31);
  assert.equal(bogota.getHours(), 10);
  assert.equal(d[Symbol.for('constructDateFrom')]('2026-06-01T00:00:00Z').timeZone, 'Asia/Tokyo');
  assert.equal(ZonedDate.tz('UTC', 0).toISOString(), '1970-01-01T00:00:00.000Z');
  assert.equal(Object.getPrototypeOf(ZonedDate), Date);
  assert.equal(typeof ZonedDate.now(), 'number');
  assert.equal(ZonedDate.UTC(2026, 0, 1), Date.UTC(2026, 0, 1));
});
testCase('a plain subclass and Reflect.construct honour new.target', () => {
  class Tagged extends Date { tag() { return 'tagged:' + this.getUTCFullYear(); } }
  const t = new Tagged(Date.UTC(2024, 1, 29));
  assert.equal(t.tag(), 'tagged:2024');
  assert.equal(t.constructor, Tagged);
  function Target() {}
  Target.prototype = Object.create(Date.prototype);
  const r = Reflect.construct(Date, [0], Target);
  assert.equal(Object.getPrototypeOf(r), Target.prototype);
  assert.equal(Date.prototype.getTime.call(r), 0);
  const now = Reflect.construct(Date, [], Tagged);
  assert.equal(now instanceof Tagged, true);
  assert.equal(Math.abs(now.getTime() - Date.now()) < 60000, true);
  assert.equal(typeof Date(), 'string');
  assert.equal(new Date(0) instanceof Date, true);
});
