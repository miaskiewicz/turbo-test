import { describe, test, expect } from 'vitest';

// typetag.node (tests/napi_support/typetag.c) is compiled by tests/napi_host.rs next to this
// file; the file runs with V8's --expose-gc so it can collect objects with finalizers.
const t = require('./typetag.node');
const gc: () => void = (globalThis as any).gc;

const OK = 0;
const INVALID_ARG = 1;
const OBJECT_EXPECTED = 2;
const PENDING_EXCEPTION = 10;
const TRUE = [OK, 1];
const FALSE = [OK, 0];

// Collect garbage and let finalizers deferred to the event loop (Node) run.
async function collect() {
  for (let i = 0; i < 3; i++) {
    gc();
    await new Promise((r) => setTimeout(r, 0));
  }
}

// The finalizer log as { id: [hint, typeof undefined] }.
function finalized(): Record<number, number[]> {
  const log = t.finalized();
  const out: Record<number, number[]> = {};
  for (let i = 0; i < log.length; i += 3) out[log[i]] = log.slice(i + 1, i + 3);
  return out;
}

describe('napi_type_tag_object / napi_check_object_type_tag', () => {
  test('a tagged object matches its tag only', () => {
    const o = {};
    expect(t.check(o, 0)).toEqual(FALSE);
    expect(t.tag(o, 0)).toBe(OK);
    expect(t.check(o, 0)).toEqual(TRUE);
    // same lower half, different upper half
    expect(t.check(o, 1)).toEqual(FALSE);
    expect(t.check({}, 0)).toEqual(FALSE);
    // an object carries one type tag
    expect(t.tag(o, 0)).toBe(INVALID_ARG);
    expect(t.tag(o, 1)).toBe(INVALID_ARG);
    expect(t.check(o, 0)).toEqual(TRUE);
    // the tag is invisible to JS
    expect(Reflect.ownKeys(o)).toEqual([]);
    expect(JSON.stringify(o)).toBe('{}');
  });

  test('tags with a zero upper half, and the all-zero tag', () => {
    const a = {};
    expect(t.tag(a, 2)).toBe(OK);
    expect(t.check(a, 2)).toEqual(TRUE);
    expect(t.check(a, 3)).toEqual(FALSE);
    const z = {};
    expect(t.check(z, 3)).toEqual(FALSE);
    expect(t.tag(z, 3)).toBe(OK);
    expect(t.check(z, 3)).toEqual(TRUE);
    expect(t.check(z, 2)).toEqual(FALSE);
  });

  test('any object can be tagged; primitives are converted with ToObject', () => {
    for (const v of [[], () => 1, new Proxy({}, {}), Object.create(null), new Number(1)]) {
      expect(t.tag(v, 1)).toBe(OK);
      expect(t.check(v, 1)).toEqual(TRUE);
    }
    // a primitive tags (and checks) a temporary wrapper object
    expect(t.tag(5, 0)).toBe(OK);
    expect(t.check(5, 0)).toEqual(FALSE);
    expect(t.tag('s', 0)).toBe(OK);
    // ToObject of undefined / null throws: napi_object_expected, and the TypeError reaches
    // the caller
    expect(() => t.tag(undefined, 0)).toThrow(TypeError);
    expect(() => t.check(null, 0)).toThrow(TypeError);
  });

  test('argument checks', () => {
    const o = {};
    // a NULL tag
    expect(t.tag(o, -1)).toBe(INVALID_ARG);
    expect(t.check(o, -1)).toEqual([INVALID_ARG, -1]);
    // NULL object (tag, check), NULL result, then both with an exception pending
    expect(t.nullChecks(o)).toEqual([
      INVALID_ARG,
      INVALID_ARG,
      INVALID_ARG,
      PENDING_EXCEPTION,
      PENDING_EXCEPTION,
    ]);
    expect(t.check(o, 0)).toEqual(FALSE);
  });
});

describe('napi_add_finalizer', () => {
  test('argument checks', () => {
    // NULL object, NULL finalizer, a non-object, then a valid call with an exception pending
    expect(t.addFinalizerChecks(5)).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, OK]);
    expect(t.addFinalizerChecks('str')).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, OK]);
    expect(t.addFinalizerChecks(null)).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, OK]);
    expect(t.addFinalizerChecks(() => 1)[2]).toBe(OK);
  });

  test('the result reference refers to the object', () => {
    const o = {};
    expect(t.addFinalizer(o, 1, 1)).toBe(OK);
    expect(t.refValue(1)).toBe(o);
    expect(Reflect.ownKeys(o)).toEqual([]);
  });

  test('finalizers run once their object is collected, several per object', async () => {
    (() => {
      const shared = {};
      expect(t.addFinalizer(shared, 11, 0)).toBe(OK);
      expect(t.addFinalizer(shared, 12, 1)).toBe(OK);
      expect(t.addFinalizer({}, 13, 2)).toBe(OK);
      expect(t.addFinalizer([], 14, 1)).toBe(OK);
    })();
    await collect();
    const log = finalized();
    // [hint, typeof undefined inside the finalizer]
    expect(log[11]).toEqual([111, 0]);
    expect(log[12]).toEqual([112, 0]);
    expect(log[13]).toEqual([113, 0]);
    expect(log[14]).toEqual([114, 0]);
    // a reference kept past the finalizer no longer has a value
    expect(t.refValue(12)).toBe(undefined);
    expect(t.refValue(14)).toBe(undefined);
  });
});
