import { describe, test, expect } from 'vitest';

// api.node (tests/napi_support/api.c) is compiled by tests/napi_host.rs next to this file. The
// expected values are Node 26's: the same spec passes against Node's own Node-API.
const api = require('./api.node');

const OK = 0;
const INVALID_ARG = 1;
const STRING_EXPECTED = 3;
const NUMBER_EXPECTED = 6;
const OBJECT_EXPECTED = 2;
const NAME_EXPECTED = 4;
const FUNCTION_EXPECTED = 5;
const GENERIC_FAILURE = 9;
const PENDING_EXCEPTION = 10;
const BIGINT_EXPECTED = 17;
const DATE_EXPECTED = 18;
const ARRAYBUFFER_EXPECTED = 19;
const DETACHABLE_ARRAYBUFFER_EXPECTED = 20;
const ESCAPE_CALLED_TWICE = 12;
const HANDLE_SCOPE_MISMATCH = 13;

// what `f` throws
function thrown(f: () => unknown): any {
  try {
    f();
  } catch (e) {
    return e;
  }
  throw new Error('did not throw');
}

describe('errors', () => {
  const CLASSES = [Error, TypeError, RangeError, SyntaxError];

  test('napi_throw_error / _type_error / _range_error / node_api_throw_syntax_error', () => {
    CLASSES.forEach((Cls, kind) => {
      const e = thrown(() => api.throwKind(kind, 'E_CODE', `kind ${kind}`));
      expect(e).toBeInstanceOf(Cls);
      expect(e.constructor).toBe(Cls);
      expect(e.message).toBe(`kind ${kind}`);
      expect(e.code).toBe('E_CODE');
      const plain = thrown(() => api.throwKind(kind, undefined, 'no code'));
      expect(plain).toBeInstanceOf(Cls);
      expect('code' in plain).toBe(false);
    });
  });

  test('napi_create_error / _type_error / _range_error / node_api_create_syntax_error', () => {
    CLASSES.forEach((Cls, kind) => {
      const [st, e] = api.createKind(kind, 'E_CODE', 'made');
      expect(st).toBe(OK);
      expect(e.constructor).toBe(Cls);
      expect([e.message, e.code]).toEqual(['made', 'E_CODE']);
      const [st2, e2] = api.createKind(kind, undefined, 'plain');
      expect(st2).toBe(OK);
      expect('code' in e2).toBe(false);
      // the message and the code must be strings
      expect(api.createKind(kind, undefined, 5)).toEqual([STRING_EXPECTED, undefined]);
      expect(api.createKind(kind, 5, 'm')).toEqual([STRING_EXPECTED, undefined]);
    });
  });

  test('a NULL message or result is napi_invalid_arg', () => {
    const [statuses, last] = api.errorChecks();
    expect(statuses).toEqual(Array(12).fill(INVALID_ARG));
    expect(last).toEqual([INVALID_ARG, 'Invalid argument', 0]);
  });

  test('napi_get_last_error_info reports the status of the last call', () => {
    expect(api.lastErrorInfo('not a number')).toEqual([
      [NUMBER_EXPECTED, 'A number was expected', 0],
      [NUMBER_EXPECTED, 'A number was expected', 0],
      [OK, undefined, 0],
      INVALID_ARG,
      [INVALID_ARG, 'Invalid argument', 0],
    ]);
  });

  test('napi_fatal_exception reports the error as uncaught', () => {
    const seen: string[] = [];
    const onUncaught = (e: Error) => seen.push(e.message);
    (globalThis as any).process.on('uncaughtException', onUncaught);
    try {
      expect(api.fatalException(new Error('reported as uncaught'))).toBe(OK);
      expect(api.fatalException(undefined)).toBe(INVALID_ARG);
    } finally {
      (globalThis as any).process.off('uncaughtException', onUncaught);
    }
  });
});

describe('numbers', () => {
  test('napi_create_int32 / napi_create_int64 (a Number, the nearest double)', () => {
    const [i32, i64, st] = api.numbers();
    expect(i32).toEqual([0, -1, -2147483648, 2147483647]);
    expect(i64).toEqual([0, -7, 9007199254740992, -9223372036854775808, 9223372036854775807]);
    expect(st).toEqual([INVALID_ARG, INVALID_ARG]);
  });

  test('napi_get_value_int32 is ToInt32 of a number', () => {
    const cases: [unknown, number[]][] = [
      [7, [OK, 7]],
      [-1.9, [OK, -1]],
      [1.9, [OK, 1]],
      [2 ** 31, [OK, -(2 ** 31)]],
      [2 ** 32 + 5, [OK, 5]],
      [1e20, [OK, 1661992960]],
      [-0, [OK, 0]],
      [NaN, [OK, 0]],
      [Infinity, [OK, 0]],
      ['1', [NUMBER_EXPECTED, -99]],
      [1n, [NUMBER_EXPECTED, -99]],
    ];
    for (const [v, want] of cases) expect(api.getInt32(v)).toEqual(want);
  });
});

describe('bigints', () => {
  test('napi_create_bigint_int64 / _uint64 / _words', () => {
    const [i, u, w, st, exc] = api.bigints();
    expect(i).toEqual([0n, -1n, -(2n ** 63n), 2n ** 63n - 1n]);
    expect(u).toEqual([0n, 1n, 2n ** 64n - 1n]);
    expect(w).toEqual([2n * 2n ** 64n + 1n, -(2n * 2n ** 64n + 1n), 0n, 1n]);
    // NULL words / result, a count over INT_MAX, over V8's maximum size (a RangeError), with an
    // exception pending, then a creation that runs no JS
    expect(st).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      PENDING_EXCEPTION, PENDING_EXCEPTION, OK]);
    expect(exc).toBeInstanceOf(RangeError);
  });

  test('napi_get_value_bigint_int64 / _uint64 report whether the value fit', () => {
    expect(api.getBigInt64(-5n)).toEqual([OK, -5n, true]);
    expect(api.getBigInt64(2n ** 63n)).toEqual([OK, -(2n ** 63n), false]);
    expect(api.getBigInt64(2n ** 64n + 3n)).toEqual([OK, 3n, false]);
    expect(api.getBigUint64(2n ** 64n - 1n)).toEqual([OK, 2n ** 64n - 1n, true]);
    expect(api.getBigUint64(-1n)).toEqual([OK, 2n ** 64n - 1n, false]);
    expect(api.getBigInt64(5)).toEqual([BIGINT_EXPECTED, 0n, false]);
  });

  test('napi_get_value_bigint_words', () => {
    const big = -(3n * 2n ** 128n + 2n * 2n ** 64n + 1n);
    expect(api.getWords(big, -1)).toEqual([OK, -1, 3, 0n, 0n, 0n, 0n]);
    expect(api.getWords(big, 4)).toEqual([OK, 1, 3, 1n, 2n, 3n, 0n]);
    // a buffer too small takes the low words; the count is the BigInt's
    expect(api.getWords(big, 2)).toEqual([OK, 1, 3, 1n, 2n, 0n, 0n]);
    expect(api.getWords(5n, 4)).toEqual([OK, 0, 1, 5n, 0n, 0n, 0n]);
    expect(api.getWords(0n, 4)).toEqual([OK, 0, 0, 0n, 0n, 0n, 0n]);
    expect(api.getWords(0n, -1)).toEqual([OK, -1, 0, 0n, 0n, 0n, 0n]);
  });

  test('argument checks', () => {
    expect(api.bigintChecks(1n)).toEqual([
      INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG, INVALID_ARG, BIGINT_EXPECTED, BIGINT_EXPECTED, BIGINT_EXPECTED,
      // the word count is checked before the type
      INVALID_ARG,
    ]);
  });
});

describe('dates', () => {
  test('napi_create_date / napi_is_date / napi_get_date_value', () => {
    const [d, isDate, otherIs, value, st] = api.dates(1700000000123.9, {});
    expect(d).toBeInstanceOf(Date);
    expect(d.getTime()).toBe(1700000000123);
    expect([isDate, otherIs, value]).toEqual([true, false, 1700000000123]);
    // created, value, a non-date, NULL value / result, napi_is_date NULL value / result,
    // napi_create_date NULL result
    expect(st).toEqual([OK, OK, DATE_EXPECTED, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG]);
    // TimeClip: out of range is an invalid date
    const [bad, , , badValue] = api.dates(8.64e15 + 1, new Date(0));
    expect(Number.isNaN(bad.getTime())).toBe(true);
    expect(Number.isNaN(badValue)).toBe(true);
    expect(api.dates(0, new Date(0))[2]).toBe(true);
  });
});

describe('strings', () => {
  test('napi_create_string_latin1 / _utf16 and node_api_create_property_key_*', () => {
    const [l1, l1cut, u16, u16cut, k1, k8, k16, st] = api.strings();
    expect([l1, l1cut]).toEqual(['café', 'caf']);
    expect([u16, u16cut]).toEqual(['hé😀', 'hé']);
    expect([k1, k8, k16]).toEqual(['kéy', 'kéy', 'hé😀']);
    // a NULL string of length 0 is the empty string; of length 1 (or NAPI_AUTO_LENGTH), a NULL
    // result or a length over INT_MAX are napi_invalid_arg
    expect(st).toEqual([OK, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, OK, INVALID_ARG,
      INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG]);
  });

  test('napi_get_value_string_latin1 / _utf16 copy code units', () => {
    expect(api.getLatin1('café', -1)).toEqual([OK, '', 4]);
    expect(api.getLatin1('café', 16)).toEqual([OK, 'café\0', 4]);
    expect(api.getLatin1('café', 3)).toEqual([OK, 'ca\0', 2]);
    expect(api.getLatin1('café', 0)).toEqual([OK, '', 0]);
    // a code unit past Latin-1 keeps its low byte
    expect(api.getLatin1('€', 16)).toEqual([OK, '\xac\0', 1]);
    expect(api.getUtf16('h😀', -1)).toEqual([OK, '', 3]);
    expect(api.getUtf16('h😀', 16)).toEqual([OK, 'h😀\0', 3]);
    // a cut may split a surrogate pair
    expect(api.getUtf16('h😀', 3)).toEqual([OK, 'h\ud83d\0', 2]);
    expect(api.getUtf16('h😀', 0)).toEqual([OK, '', 0]);
    expect(api.getStringChecks(5)).toEqual([INVALID_ARG, STRING_EXPECTED, INVALID_ARG, OK,
      INVALID_ARG, STRING_EXPECTED, INVALID_ARG, OK]);
  });

  test('node_api_create_external_string_latin1 / _utf16: the finalizer runs on collection', async () => {
    const before = api.extFinalized();
    // (in a function of its own, so nothing holds the strings afterwards)
    (() => {
      const [l1, u16, fixed, st] = api.externalStrings();
      expect(l1).toEqual([OK, 'external ééé', false]);
      expect(u16).toEqual([OK, '€😀', false]);
      expect(fixed).toBe('fixed');
      expect(st).toEqual([INVALID_ARG, INVALID_ARG]);
    })();
    expect(api.extFinalized()).toBe(before);
    for (let i = 0; i < 3; i++) {
      (globalThis as any).gc();
      await new Promise((r) => setTimeout(r, 0));
    }
    expect(api.extFinalized()).toBe(before + 2);
  });
});

describe('symbols, coercions, arrays', () => {
  test('napi_create_symbol / node_api_symbol_for', () => {
    const [st, sym, symFor, checks] = api.symbols('desc');
    expect(st).toBe(OK);
    expect(typeof sym).toBe('symbol');
    expect(sym.description).toBe('desc');
    expect(symFor).toBe(Symbol.for('tt.key'));
    expect(checks).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG]);
    const [st2, plain] = api.symbols(undefined);
    expect([st2, plain.description]).toEqual([OK, undefined]);
    expect(api.symbols(5).slice(0, 2)).toEqual([STRING_EXPECTED, undefined]);
  });

  test('napi_coerce_to_bool / _number / _object', () => {
    const [b, n, e1, o, e2] = api.coerce('12');
    expect([b, n, e1, e2]).toEqual([[OK, true], [OK, 12], undefined, undefined]);
    expect(o[0]).toBe(OK);
    expect(o[1]).toBeInstanceOf(String);
    expect(api.coerce(0)[0]).toEqual([OK, false]);
    // ToNumber of a symbol throws (napi_number_expected, the TypeError pending)
    const [, sn, se] = api.coerce(Symbol('s'));
    expect(sn).toEqual([NUMBER_EXPECTED, undefined]);
    expect(se).toBeInstanceOf(TypeError);
    // ToObject of null throws
    const [nb, nn, , no, ne] = api.coerce(null);
    expect([nb, nn]).toEqual([[OK, false], [OK, 0]]);
    expect(no).toEqual([OBJECT_EXPECTED, undefined]);
    expect(ne).toBeInstanceOf(TypeError);
    expect(api.coerceChecks()).toEqual(Array(6).fill(INVALID_ARG));
  });

  test('napi_create_array', () => {
    const [arr, st] = api.createArray();
    expect(Array.isArray(arr)).toBe(true);
    expect(arr.length).toBe(0);
    expect(st).toBe(INVALID_ARG);
  });
});

describe('properties', () => {
  test('set / get / has / has own / has named / delete by a key value', () => {
    const o: any = { named: 1 };
    const sym = Symbol('k');
    const [set, get, has, own, named, del, after, exc] = api.props(o, 'k', 7);
    expect([set, get, has, own, named, del, after]).toEqual([
      [OK, undefined], [OK, 7], [OK, true], [OK, true], [OK, true], [OK, true], [OK, false],
    ]);
    expect(exc).toBe(undefined);
    expect(api.props(o, sym, 'v').slice(0, 7)).toEqual([
      [OK, undefined], [OK, 'v'], [OK, true], [OK, true], [OK, true], [OK, true], [OK, false],
    ]);
    // a number key works for every call but napi_has_own_property (which takes a name)
    const child: any = {};
    const r = api.props(child, 3, 'three');
    expect(r.slice(0, 7)).toEqual([
      [OK, undefined], [OK, 'three'], [OK, true], [NAME_EXPECTED, false], [OK, false],
      [OK, true], [OK, false],
    ]);
    expect(3 in child).toBe(false);
    // a non-configurable property is not deleted (result false, status napi_ok)
    const fixed = Object.defineProperty({}, 'k', { value: 1 });
    const f = api.props(fixed, 'k', 2);
    expect([f[0], f[1], f[5], f[6]]).toEqual([[OK, undefined], [OK, 1], [OK, false], [OK, true]]);
  });

  test('a throwing accessor or trap leaves its exception pending', () => {
    const boom = new Proxy({}, {
      get() { throw new Error('get'); },
      set() { throw new Error('set'); },
      has() { throw new Error('has'); },
      deleteProperty() { throw new Error('delete'); },
      getOwnPropertyDescriptor() { throw new Error('own'); },
    });
    const r = api.props(boom, 'k', 1);
    expect(r.slice(0, 7).map((p: any[]) => p[0])).toEqual(Array(7).fill(PENDING_EXCEPTION));
    expect(r[7].message).toBe('set');
    // a set trap returning false: a sloppy-mode Set, which succeeds
    const refuses = new Proxy({}, { set: () => false });
    expect(api.props(refuses, 'k', 1)[0]).toEqual([OK, undefined]);
  });

  test('argument checks', () => {
    expect(api.propChecks({})).toEqual([
      INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, OK, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, NAME_EXPECTED, INVALID_ARG, INVALID_ARG, INVALID_ARG,
    ]);
    // ToObject of null: napi_object_expected, the TypeError pending
    const r = api.props(null, 'k', 1);
    expect(r[0]).toEqual([OBJECT_EXPECTED, undefined]);
    expect(r[7]).toBeInstanceOf(TypeError);
  });

  test('napi_has_element / napi_delete_element', () => {
    const arr = [1, 2];
    expect(api.elements(arr, 1).slice(0, 3)).toEqual([[OK, true], [OK, true], [OK, false]]);
    expect(arr.length).toBe(2);
    expect(api.elements(arr, 5).slice(0, 3)).toEqual([[OK, false], [OK, true], [OK, false]]);
    const frozen = Object.freeze([1]);
    expect(api.elements(frozen, 0).slice(0, 3)).toEqual([[OK, true], [OK, false], [OK, true]]);
    expect(api.elementChecks([])).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, OK]);
    const [, , , exc] = api.elements(undefined, 0);
    expect(exc).toBeInstanceOf(TypeError);
  });

  test('napi_get_all_property_names: mode, filter, conversion', () => {
    const proto = { inherited: 1 };
    const o = Object.create(proto);
    const sym = Symbol('s');
    Object.defineProperty(o, 'hidden', { value: 1, enumerable: false, writable: true });
    Object.defineProperty(o, 'ro', { value: 1, enumerable: true, configurable: true });
    o[2] = 'two';
    o.plain = 1;
    o[sym] = 1;
    const names = (mode: number, filter: number, conv: number) => {
      const [st, n] = api.allNames(o, mode, filter, conv);
      expect(st).toBe(OK);
      return n;
    };
    expect(names(1, 0, 1)).toEqual(['2', 'hidden', 'ro', 'plain', sym]);
    expect(names(1, 0, 0)).toEqual([2, 'hidden', 'ro', 'plain', sym]);
    expect(names(0, 2, 1)).toEqual(['2', 'ro', 'plain', sym, 'inherited']);
    expect(names(1, 1, 1)).toEqual(['2', 'hidden', 'plain', sym]);
    expect(names(1, 4, 1)).toEqual(['2', 'ro', 'plain', sym]);
    expect(names(1, 8, 1)).toEqual([sym]);
    expect(names(1, 16, 1)).toEqual(['2', 'hidden', 'ro', 'plain']);
    expect(names(1, 2 | 16, 0)).toEqual([2, 'ro', 'plain']);
    const trap = new Proxy({}, { ownKeys() { throw new Error('keys'); } });
    const [st, , exc] = api.allNames(trap, 1, 0, 1);
    expect(st).toBe(PENDING_EXCEPTION);
    expect(exc.message).toBe('keys');
  });

  test('napi_get_prototype, napi_object_freeze / napi_object_seal', () => {
    const proto = {};
    expect(api.proto(Object.create(proto)).slice(0, 2)).toEqual([OK, proto]);
    expect(api.proto(Object.create(null)).slice(0, 2)).toEqual([OK, null]);
    expect(api.proto(5).slice(0, 2)).toEqual([OK, Number.prototype]);
    // a Proxy's getPrototypeOf trap is not run (V8's GetPrototypeV2 gives a proxy null)
    const p = new Proxy({}, { getPrototypeOf() { throw new Error('trap'); } });
    expect(api.proto(p)).toEqual([OK, null, undefined]);
    const f: any = { a: 1 };
    expect(api.integrity(f, false)).toEqual([OK, undefined]);
    expect(Object.isFrozen(f)).toBe(true);
    const sl: any = { a: 1 };
    expect(api.integrity(sl, true)).toEqual([OK, undefined]);
    expect([Object.isSealed(sl), Object.isFrozen(sl)]).toEqual([true, false]);
    const no = new Proxy({}, { preventExtensions() { throw new Error('no'); } });
    const [st, exc] = api.integrity(no, false);
    expect(st).toBe(PENDING_EXCEPTION);
    expect(exc.message).toBe('no');
    const refuses = new Proxy({}, { preventExtensions: () => false });
    const [st2, exc2] = api.integrity(refuses, true);
    expect([st2, exc2 && exc2.constructor]).toEqual([PENDING_EXCEPTION, TypeError]);
    expect(api.objectChecks()).toEqual(Array(8).fill(INVALID_ARG));
  });
});

describe('comparisons and type checks', () => {
  test('napi_strict_equals is ===', () => {
    const o = {};
    expect(api.compare(o, o)).toEqual([OK, true]);
    expect(api.compare(o, {})).toEqual([OK, false]);
    expect(api.compare(NaN, NaN)).toEqual([OK, false]);
    expect(api.compare(0, -0)).toEqual([OK, true]);
    expect(api.compare('a', 'a')).toEqual([OK, true]);
    expect(api.compare(1, '1')).toEqual([OK, false]);
  });

  test('napi_instanceof', () => {
    class A {}
    class B extends A {}
    expect(api.instanceOf(new B(), A)).toEqual([OK, true, undefined]);
    expect(api.instanceOf(new A(), B)).toEqual([OK, false, undefined]);
    expect(api.instanceOf(5, Number)).toEqual([OK, false, undefined]);
    // Symbol.hasInstance decides, but only on a function
    const Any = { [Symbol.hasInstance]: () => true };
    expect(api.instanceOf(1, Any)[0]).toBe(FUNCTION_EXPECTED);
    const Yes = function () {};
    Object.defineProperty(Yes, Symbol.hasInstance, { value: () => true });
    expect(api.instanceOf(1, Yes)).toEqual([OK, true, undefined]);
    // a constructor that is not a function: a TypeError and napi_function_expected
    const [st, r, exc] = api.instanceOf({}, {});
    expect([st, r]).toEqual([FUNCTION_EXPECTED, false]);
    expect(exc).toBeInstanceOf(TypeError);
    expect([exc.message, exc.code]).toEqual(['Constructor must be a function', 'ERR_NAPI_CONS_FUNCTION']);
    // ToObject of null: napi_object_expected
    expect(api.instanceOf({}, null).slice(0, 2)).toEqual([OBJECT_EXPECTED, false]);
    // a throwing Symbol.hasInstance: napi_generic_failure, the exception pending
    const Throws = function () {};
    Object.defineProperty(Throws, Symbol.hasInstance, { value: () => { throw new Error('has'); } });
    const [st2, , exc2] = api.instanceOf({}, Throws);
    expect(st2).toBe(GENERIC_FAILURE);
    expect(exc2.message).toBe('has');
  });

  test('napi_is_arraybuffer / _typedarray / _dataview / _buffer / _detached_arraybuffer', () => {
    const ab = new ArrayBuffer(8);
    expect(api.kinds(ab)).toEqual([true, false, false, false, false, OK]);
    expect(api.kinds(new Uint8Array(ab))).toEqual([false, true, false, true, false, OK]);
    expect(api.kinds(new DataView(ab))).toEqual([false, false, true, true, false, OK]);
    expect(api.kinds({})).toEqual([false, false, false, false, false, OK]);
    const detached = new ArrayBuffer(4);
    (detached as any).transfer();
    expect(api.kinds(detached)).toEqual([true, false, false, false, true, OK]);
    expect(api.compareChecks()).toEqual(Array(12).fill(INVALID_ARG));
  });
});

describe('arraybuffers, typed arrays, dataviews, buffers', () => {
  test('napi_get_arraybuffer_info (ArrayBuffer and SharedArrayBuffer)', () => {
    const ab = new Uint8Array([5, 6]).buffer;
    expect(api.abInfo(ab)).toEqual([OK, 2, 5]);
    expect(new Uint8Array(ab)[0]).toBe(6); // the data pointer is the buffer's memory
    const sab = new SharedArrayBuffer(3);
    expect(api.abInfo(sab)).toEqual([OK, 3, 0]);
    expect(new Uint8Array(sab)[0]).toBe(1);
    expect(api.abInfo(new ArrayBuffer(0))).toEqual([OK, 0, -1]);
    expect(api.abInfo(new Uint8Array(2))).toEqual([INVALID_ARG, 999, -1]);
  });

  test('napi_get_typedarray_info: type, length, offset, buffer, data', () => {
    const ab = new ArrayBuffer(16);
    const kinds: [any, number][] = [
      [Int8Array, 0], [Uint8Array, 1], [Uint8ClampedArray, 2], [Int16Array, 3],
      [Uint16Array, 4], [Int32Array, 5], [Uint32Array, 6], [Float32Array, 7],
      [Float64Array, 8], [BigInt64Array, 9], [BigUint64Array, 10], [Float16Array, 11],
    ];
    for (const [Ctor, type] of kinds) {
      const [st, t, len, off, buf] = api.taInfo(new Ctor(ab, 8, 1));
      expect([st, t, len, off, buf === ab]).toEqual([OK, type, 1, 8, true]);
    }
    const u = new Uint8Array([1, 2, 3, 4]);
    expect(api.taInfo(u.subarray(2)).filter((_: unknown, i: number) => i !== 4)).toEqual([OK, 1, 2, 2, 3]);
    expect(u[2]).toBe(4);
    expect(api.taInfo(new DataView(ab))[0]).toBe(INVALID_ARG);
  });

  test('the buffer creators hand back Buffers', () => {
    const [b, copy, ext, fromAb] = api.buffers();
    for (const x of [b, copy, ext, fromAb]) {
      expect(Buffer.isBuffer(x)).toBe(true);
      expect(x).toBeInstanceOf(Uint8Array);
    }
    expect([b.length, copy.toString(), ext.toString('latin1'), fromAb.length]).toEqual([2, 'abc', 'ext', 2]);
  });

  test('napi_create_typedarray over an ArrayBuffer or a SharedArrayBuffer', () => {
    for (const buffer of [new ArrayBuffer(8), new SharedArrayBuffer(8)]) {
      const [st, ta, exc] = api.typedArrayOver(buffer, 5, 1, 4);
      expect([st, exc]).toEqual([OK, undefined]);
      expect(ta).toBeInstanceOf(Int32Array);
      expect([ta.buffer === buffer, ta.byteOffset, ta.length]).toEqual([true, 4, 1]);
      // the same alignment and length checks
      const [st2, , exc2] = api.typedArrayOver(buffer, 5, 1, 2);
      expect([st2, exc2.code]).toEqual([GENERIC_FAILURE, 'ERR_NAPI_INVALID_TYPEDARRAY_ALIGNMENT']);
      const [st3, , exc3] = api.typedArrayOver(buffer, 5, 2, 4);
      expect([st3, exc3.code]).toEqual([GENERIC_FAILURE, 'ERR_NAPI_INVALID_TYPEDARRAY_LENGTH']);
    }
    expect(api.typedArrayOver({}, 1, 1, 0).slice(0, 2)).toEqual([INVALID_ARG, undefined]);
  });

  test('napi_create_dataview / napi_get_dataview_info', () => {
    const ab = new Uint8Array([1, 2, 3, 4]).buffer;
    const [st, dv, exc] = api.dataView(ab, 2, 1);
    expect([st, exc]).toEqual([OK, undefined]);
    expect(dv).toBeInstanceOf(DataView);
    expect([dv.byteLength, dv.byteOffset, dv.buffer === ab]).toEqual([2, 1, true]);
    const [ist, len, off, buf, first] = api.dvInfo(dv);
    expect([ist, len, off, buf === ab, first]).toEqual([OK, 2, 1, true, 2]);
    expect(new Uint8Array(ab)[1]).toBe(3);
    // over the end: a RangeError and napi_pending_exception
    const [st2, dv2, exc2] = api.dataView(ab, 4, 1);
    expect([st2, dv2]).toEqual([PENDING_EXCEPTION, undefined]);
    expect(exc2).toBeInstanceOf(RangeError);
    expect(exc2.code).toBe('ERR_NAPI_INVALID_DATAVIEW_ARGS');
    expect(exc2.message).toBe(
      'byte_offset + byte_length should be less than or equal to the size in bytes of the array passed in',
    );
    const sab = new SharedArrayBuffer(4);
    const [st3, dv3] = api.dataView(sab, 4, 0);
    expect([st3, dv3.buffer === sab]).toEqual([OK, true]);
    expect(api.dvInfo(new Uint8Array(1))[0]).toBe(INVALID_ARG);
  });

  test('napi_detach_arraybuffer', () => {
    const ab = new ArrayBuffer(8);
    expect(api.detach(ab)).toBe(OK);
    expect(ab.byteLength).toBe(0);
    expect(api.kinds(ab)[4]).toBe(true);
    expect(api.detach(new Uint8Array(1))).toBe(ARRAYBUFFER_EXPECTED);
    expect(api.detach(new SharedArrayBuffer(1))).toBe(ARRAYBUFFER_EXPECTED);
    // WebAssembly.Memory's buffer cannot be detached
    const mem = new WebAssembly.Memory({ initial: 1 });
    expect(api.detach(mem.buffer)).toBe(DETACHABLE_ARRAYBUFFER_EXPECTED);
  });

  test('node_api_create_buffer_from_arraybuffer', () => {
    const ab = new Uint8Array([1, 2, 3, 4]).buffer;
    const [st, b, exc] = api.bufferFromAb(ab, 1, 2);
    expect([st, exc]).toEqual([OK, undefined]);
    expect(b).toBeInstanceOf(Uint8Array);
    expect([b.buffer === ab, b.byteOffset, [...b]]).toEqual([true, 1, [2, 3]]);
    // out of range: the RangeError is thrown, the status is napi_throw_range_error's (napi_ok)
    const [st2, b2, exc2] = api.bufferFromAb(ab, 3, 2);
    expect([st2, b2]).toEqual([OK, undefined]);
    expect(exc2).toBeInstanceOf(RangeError);
    expect([exc2.code, exc2.message]).toEqual(['ERR_OUT_OF_RANGE', 'The byte offset + length is out of range']);
    expect(api.viewChecks()).toEqual([INVALID_ARG, INVALID_ARG, OK, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG]);
  });

  test('external ArrayBuffers and Buffers alias the addon memory; the finalizer runs on collection', async () => {
    const before = api.extAbFinalized();
    for (const buffer of [false, true]) {
      (() => {
        const ext = api.externalAb(buffer);
        const bytes = buffer ? ext : new Uint8Array(ext);
        expect(buffer ? ext instanceof Uint8Array : ext instanceof ArrayBuffer).toBe(true);
        expect([...bytes]).toEqual([7, 0, 0, 0]);
        // writes on either side are seen by the other
        api.extAbPoke(9);
        expect(bytes[0]).toBe(9);
        bytes[1] = 5;
        expect(api.abInfo(buffer ? ext.buffer : ext)).toEqual([OK, 4, 9]);
        expect(bytes[0]).toBe(10);
      })();
      for (let i = 0; i < 3; i++) {
        (globalThis as any).gc();
        await new Promise((r) => setTimeout(r, 0));
      }
    }
    expect(api.extAbFinalized()).toBe(before + 2);
    expect(api.externalAbNoResult()).toBe(OK);
  });
});

describe('promises and externals', () => {
  test('napi_create_promise / napi_resolve_deferred / napi_reject_deferred / napi_is_promise', async () => {
    const [st, p, isPromise, done] = api.promise(true, 'value');
    expect([st, isPromise, done]).toEqual([OK, true, OK]);
    expect(p).toBeInstanceOf(Promise);
    expect(await p).toBe('value');
    const [st2, p2, , done2] = api.promise(false, new Error('nope'));
    expect([st2, done2]).toEqual([OK, OK]);
    let rejected: any;
    await p2.catch((e: unknown) => { rejected = e; });
    expect(rejected.message).toBe('nope');
    // a deferred settled later, from another call
    const later = api.deferLater();
    expect(api.settleLater(true, 42)).toBe(OK);
    expect(await later).toBe(42);
    const later2 = api.deferLater();
    expect(api.settleLater(false, 'why')).toBe(OK);
    let reason;
    await later2.catch((e: unknown) => { reason = e; });
    expect(reason).toBe('why');
    const [isP, checks] = api.promiseChecks({ then() {} });
    expect(isP).toBe(false);
    expect(checks).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      PENDING_EXCEPTION, PENDING_EXCEPTION, OK]);
  });

  test('napi_create_external / napi_get_value_external; the finalizer runs on collection', async () => {
    const before = api.externalFinalized();
    (() => {
      const [st, e, type, dataMatches, e2, checks] = api.externals();
      expect([st, type, dataMatches]).toEqual([OK, 8, true]);
      expect(typeof e).toBe('object');
      expect(Object.getPrototypeOf(e)).toBe(null);
      expect(e).not.toBe(e2);
      expect(checks).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG]);
    })();
    for (let i = 0; i < 3; i++) {
      (globalThis as any).gc();
      await new Promise((r) => setTimeout(r, 0));
    }
    expect(api.externalFinalized()).toBe(before + 1);
  });

  test('an External carries one type tag', () => {
    const e = api.externals()[1];
    // check before tagging: false; tag; a second tag is napi_invalid_arg; check T1, T2
    expect(api.externalTags(e)).toEqual([OK, OK, INVALID_ARG, OK, OK, 1, 0, 0]);
  });

  // host-only: Node dereferences a NULL tag or result for an External
  test('host-only: a NULL tag or result for an External is napi_invalid_arg', () => {
    expect(api.externalTagNulls(api.externals()[1])).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG]);
  });
});

describe('instances, scripts, handle scopes, memory, module file name', () => {
  test('napi_new_instance', () => {
    class P {
      sum: number;
      constructor(a: number, b: number) { this.sum = a + b; }
    }
    const [st, p, exc] = api.newInstance(P, 2, 3);
    expect([st, exc]).toEqual([OK, undefined]);
    expect(p).toBeInstanceOf(P);
    expect(p.sum).toBe(5);
    expect(api.newInstance({}, 1, 2)).toEqual([INVALID_ARG, undefined, undefined]);
    class Throws { constructor() { throw new Error('ctor'); } }
    const [st2, , exc2] = api.newInstance(Throws, 1, 2);
    expect(st2).toBe(PENDING_EXCEPTION);
    expect(exc2.message).toBe('ctor');
    expect(api.newInstanceChecks(P)).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, OK]);
  });

  test('napi_run_script', () => {
    expect(api.runScript('6 * 7')).toEqual([OK, 42, undefined]);
    (globalThis as any).fromScript = undefined;
    expect(api.runScript('globalThis.fromScript = "set"; 1')).toEqual([OK, 1, undefined]);
    expect((globalThis as any).fromScript).toBe('set');
    expect(api.runScript(5)).toEqual([STRING_EXPECTED, undefined, undefined]);
    // a syntax error or a throwing script: napi_generic_failure, the exception pending
    const [st, , exc] = api.runScript('(');
    expect(st).toBe(GENERIC_FAILURE);
    expect(exc).toBeInstanceOf(SyntaxError);
    const [st2, , exc2] = api.runScript('throw new RangeError("ran")');
    expect(st2).toBe(GENERIC_FAILURE);
    expect(exc2).toBeInstanceOf(RangeError);
  });

  test('handle scopes: open / close, escape once, a close too many', () => {
    const v = { escaping: true };
    const [st, escaped, again, last] = api.handleScopes(v);
    expect(st).toEqual([OK, OK, OK, OK, ESCAPE_CALLED_TWICE, OK, HANDLE_SCOPE_MISMATCH,
      HANDLE_SCOPE_MISMATCH, INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG]);
    expect(escaped).toBe(v);
    expect(again).toBe(undefined);
    expect(last).toEqual([NUMBER_EXPECTED, 'A number was expected', 0]);
    expect(api.escapeChecks(v)).toEqual([INVALID_ARG, INVALID_ARG]);
  });

  test('napi_adjust_external_memory, node_api_get_module_file_name', () => {
    expect(api.externalMemory(1024)).toEqual([OK, 1, INVALID_ARG]);
    const [st, name, nullResult] = api.moduleFileName();
    expect([st, nullResult]).toEqual([OK, INVALID_ARG]);
    // the addon's absolute path as a file: URL
    expect(name.startsWith('file:///')).toBe(true);
    expect(name.endsWith('/api.node')).toBe(true);
  });
});

describe('NULL arguments of the core entrypoints', () => {
  test('a NULL result or value is napi_invalid_arg', () => {
    const st = api.coreNulls();
    const want = Array(st.length).fill(INVALID_ARG);
    // napi_call_function's result is optional
    want[30] = OK;
    expect(st).toEqual(want);
  });
});

describe('finalizers', () => {
  test('an exception a finalizer leaves is uncaught', async () => {
    const seen: string[] = [];
    const onUncaught = (e: Error) => seen.push(e.message);
    (globalThis as any).process.on('uncaughtException', onUncaught);
    try {
      expect((() => api.throwingFinalizer({}))()).toBe(OK);
      for (let i = 0; i < 3; i++) {
        (globalThis as any).gc();
        await new Promise((r) => setTimeout(r, 0));
      }
      // (Node: process 'uncaughtException'; turbo-test reports it on stderr, as a timer's)
      (globalThis as any).__seenUncaught = seen;
    } finally {
      (globalThis as any).process.off('uncaughtException', onUncaught);
    }
  });
});

describe('a NULL env', () => {
  test('is napi_invalid_arg for every entrypoint', () => {
    const statuses = api.nullEnv();
    expect(statuses.length).toBeGreaterThan(0);
    expect(statuses).toEqual(Array(statuses.length).fill(INVALID_ARG));
  });
});
