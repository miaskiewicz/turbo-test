import { describe, test, expect } from 'vitest';

// wrap.node (tests/napi_support/wrap.c) is compiled by tests/napi_host.rs next to this file;
// the file runs with V8's --expose-gc so it can collect wrapped objects.
const w = require('./wrap.node');
const gc: () => void = (globalThis as any).gc;

const OK = 0;
const INVALID_ARG = 1;

// Collect garbage and let finalizers deferred to the event loop (Node) run.
async function collect() {
  for (let i = 0; i < 3; i++) {
    gc();
    await new Promise((r) => setTimeout(r, 0));
  }
}

// The finalizer log as { id: [hint, typeof undefined, delete status] }.
function finalized(): Record<number, number[]> {
  const log = w.finalized();
  const out: Record<number, number[]> = {};
  for (let i = 0; i < log.length; i += 4) out[log[i]] = log.slice(i + 1, i + 4);
  return out;
}

describe('napi_wrap / napi_unwrap / napi_remove_wrap', () => {
  test('an ObjectWrap-style class: the constructor wraps, a method unwraps and mutates', () => {
    const c = new w.Counter(10);
    expect(c).toBeInstanceOf(w.Counter);
    expect(c.increment()).toBe(11);
    expect(c.increment()).toBe(12);
    const d = new w.Counter(0);
    expect(d.increment()).toBe(1);
    expect(c.increment()).toBe(13);
    // a method called on an object the constructor did not wrap: napi_unwrap fails
    expect(() => w.Counter.prototype.increment.call({})).toThrow('not a Counter');
  });

  test('wrap, unwrap, a second wrap, remove_wrap, wrap again', () => {
    const o = {};
    expect(w.unwrap(o)).toEqual([INVALID_ARG, -1]);
    expect(w.wrap(o, 7)).toBe(OK);
    expect(w.unwrap(o)).toEqual([OK, 7]);
    expect(w.unwrap(o)).toEqual([OK, 7]);
    // an object carries one wrap: a second napi_wrap is napi_invalid_arg and changes nothing
    expect(w.wrap(o, 8)).toBe(INVALID_ARG);
    expect(w.unwrap(o)).toEqual([OK, 7]);
    // remove_wrap hands the pointer back and unwraps the object
    expect(w.removeWrap(o)).toEqual([OK, 7]);
    expect(w.unwrap(o)).toEqual([INVALID_ARG, -1]);
    expect(w.removeWrap(o)).toEqual([INVALID_ARG, -1]);
    // which can then be wrapped again
    expect(w.wrap(o, 9)).toBe(OK);
    expect(w.unwrap(o)).toEqual([OK, 9]);
    // unwrap needs `result`; remove_wrap does not
    expect(w.unwrapNoResult(o)).toBe(INVALID_ARG);
    expect(w.removeWrapNoResult(o)).toBe(OK);
    expect(w.unwrap(o)).toEqual([INVALID_ARG, -1]);
    // the wrap is invisible to JS
    expect(w.wrap(o, 10)).toBe(OK);
    expect(Reflect.ownKeys(o)).toEqual([]);
    expect(JSON.stringify(o)).toBe('{}');
  });

  test('any object can be wrapped; anything else is napi_invalid_arg', () => {
    for (const v of [[], () => 1, new Proxy({}, {}), new Number(1), Object.create(null)]) {
      expect(w.wrap(v, 5)).toBe(OK);
      expect(w.unwrap(v)).toEqual([OK, 5]);
    }
    for (const v of [5, 'str', true, null, undefined, Symbol('s'), 1n]) {
      expect(w.wrap(v, 5)).toBe(INVALID_ARG);
      expect(w.unwrap(v)).toEqual([INVALID_ARG, -1]);
      expect(w.removeWrap(v)).toEqual([INVALID_ARG, -1]);
    }
    expect(w.wrapNull()).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG]);
  });

  test('the result reference needs a finalizer and refers to the wrapped object', () => {
    expect(w.wrapRefNoFin({})).toBe(INVALID_ARG);
    const o = {};
    expect(w.wrapFin(o, 3, 1)).toBe(OK);
    expect(w.refValue(3)).toBe(o);
    expect(w.unwrap(o)).toEqual([OK, 3]);
    // the reference outlives the wrap
    expect(w.removeWrap(o)).toEqual([OK, 3]);
    expect(w.refValue(3)).toBe(o);
  });

  test('a finalizer runs once its object is collected; remove_wrap drops it', async () => {
    const before = w.countersFreed();
    const removed = {};
    expect(w.wrapFin(removed, 21, 0)).toBe(OK);
    expect(w.removeWrap(removed)).toEqual([OK, 21]);
    (() => {
      expect(w.wrapFin({}, 11, 0)).toBe(OK);
      expect(w.wrapFin({}, 12, 1)).toBe(OK);
      expect(w.wrapFin({}, 13, 2)).toBe(OK);
      expect(w.wrapFin({}, 14, 0)).toBe(OK);
      expect(w.wrapFin(w.removeWrap.bind(null), 15, 0)).toBe(OK);
      new w.Counter(1).increment();
    })();
    await collect();
    const log = finalized();
    // [hint, typeof undefined inside the finalizer, napi_delete_reference status (-1: none)]
    expect(log[11]).toEqual([111, 0, -1]);
    expect(log[12]).toEqual([112, 0, -1]);
    expect(log[13]).toEqual([113, 0, OK]);
    expect(log[14]).toEqual([114, 0, -1]);
    expect(log[15]).toEqual([115, 0, -1]);
    expect(log[21]).toBe(undefined);
    expect(w.countersFreed()).toBeGreaterThan(before);
    // a reference kept past the finalizer no longer has a value
    expect(w.refValue(12)).toBe(undefined);
    expect(w.unwrap(removed)).toEqual([INVALID_ARG, -1]);
  });
});
