import { describe, test, expect } from 'vitest';

// Fixture addon compiled by tests/napi_host.rs (source: tests/napi_support/refs.c). Expected
// values are Node 26's (the same spec passes against Node's Node-API with --expose-gc).
const r = require('./refs.node');
const gc = (globalThis as any).gc;
const OK = 0;
const INVALID_ARG = 1;
const GENERIC_FAILURE = 9;

async function collect() {
  for (let i = 0; i < 3; i++) {
    gc();
    await new Promise((res) => setTimeout(res, 0));
  }
}
// a reference to an object nothing else holds
function refToFresh(count: number) {
  const [st, ref] = r.create({ fresh: true }, count);
  expect(st).toBe(OK);
  return ref;
}

describe('napi_create_reference / napi_reference_ref / napi_reference_unref', () => {
  test('the count goes up and down; unref at 0 is napi_generic_failure', () => {
    const o = { id: 1 };
    const [st, ref] = r.create(o, 2);
    expect(st).toBe(OK);
    expect(r.ref(ref)).toEqual([OK, 3]);
    expect(r.unref(ref)).toEqual([OK, 2]);
    expect(r.unref(ref)).toEqual([OK, 1]);
    expect(r.unref(ref)).toEqual([OK, 0]);
    expect(r.unref(ref)).toEqual([GENERIC_FAILURE, 77]);
    // weak, but the object is alive: the value is still there, and ref makes it strong again
    expect(r.get(ref)).toEqual([OK, o]);
    expect(r.ref(ref)).toEqual([OK, 1]);
    expect(r.get(ref)).toEqual([OK, o]);
    expect(r.del(ref)).toBe(OK);
  });

  test('a reference created with count 0 is weak: its value goes once the object is collected', async () => {
    const weak = refToFresh(0);
    const strong = refToFresh(1);
    expect(r.get(weak)[1]).toEqual({ fresh: true });
    await collect();
    expect(r.get(weak)).toEqual([OK, 'NULL']);
    expect(r.get(strong)).toEqual([OK, { fresh: true }]);
    // ref on a reference whose value is gone stays at 0; unref of 0 still fails
    expect(r.ref(weak)).toEqual([OK, 0]);
    expect(r.get(weak)).toEqual([OK, 'NULL']);
    expect(r.unref(weak)).toEqual([GENERIC_FAILURE, 77]);
    // a strong reference unref'd to 0 becomes weak and loses its value on collection
    expect(r.unref(strong)).toEqual([OK, 0]);
    await collect();
    expect(r.get(strong)).toEqual([OK, 'NULL']);
    expect(r.del(weak)).toBe(OK);
    expect(r.del(strong)).toBe(OK);
  });

  test('symbols can be held weakly; other primitives are napi_invalid_arg', () => {
    const sym = Symbol('s');
    const [st, ref] = r.create(sym, 0);
    expect(st).toBe(OK);
    expect(r.get(ref)).toEqual([OK, sym]);
    expect(r.del(ref)).toBe(OK);
    for (const v of ['str', 1, true, null, undefined, 10n]) {
      expect(r.create(v, 1)[0]).toBe(INVALID_ARG);
    }
    expect(r.create(() => 1, 1)[0]).toBe(OK);
  });

  test("a wrap's own reference made strong keeps the object (and its finalizer) alive", async () => {
    const before = r.finalized();
    const own = (() => {
      const [st, ref] = r.wrap({ wrapped: true });
      expect(st).toBe(OK);
      return ref;
    })();
    expect(r.ref(own)).toEqual([OK, 1]);
    await collect();
    expect(r.finalized()).toBe(before);
    expect(r.get(own)).toEqual([OK, { wrapped: true }]);
    expect(r.unref(own)).toEqual([OK, 0]);
    await collect();
    expect(r.finalized()).toBe(before + 1);
    expect(r.get(own)).toEqual([OK, 'NULL']);
    expect(r.del(own)).toBe(OK);
  });
});
