import { describe, test, expect } from 'vitest';

// full.node (tests/napi_support/full.c) is compiled by tests/napi_host.rs next to this file.
const a = require('./full.node');

// napi_status codes as src/napi_host.rs defines them.
const OK = 0;
const INVALID_ARG = 1;
const OBJECT_EXPECTED = 2;
const STRING_EXPECTED = 3;
const NUMBER_EXPECTED = 6;
const BOOLEAN_EXPECTED = 7;
const ARRAY_EXPECTED = 8;
const GENERIC_FAILURE = 9;
const PENDING_EXCEPTION = 10;

// `f` throws what ToObject(null / undefined) throws
function throwsToObjectError(f: () => unknown) {
  let caught: any;
  try {
    f();
  } catch (e) {
    caught = e;
  }
  expect(caught).toBeInstanceOf(TypeError);
  expect(caught.message).toBe('Cannot convert undefined or null to object');
}

describe('module init', () => {
  test('an init returning NULL hands back the exports object it was given', () => {
    expect(typeof a.basics).toBe('function');
    expect(typeof a.Cls).toBe('function');
    // cached: a second require() returns the same exports
    expect(require('./full.node')).toBe(a);
  });
});

describe('values', () => {
  test('getters and creators', () => {
    const [u, n, g, t, f, max, d, utf, cut, obj, arr, empty, acc] = a.basics();
    expect(u).toBe(undefined);
    expect(n).toBe(null);
    expect(g).toBe(globalThis);
    expect(t).toBe(true);
    expect(f).toBe(false);
    expect(max).toBe(4294967295);
    expect(d).toBe(-1.5);
    expect(utf).toBe('héllo wörld €');
    expect(cut).toBe('abc');
    expect(obj).toEqual({});
    expect(Object.getPrototypeOf(obj)).toBe(Object.prototype);
    expect(Array.isArray(arr)).toBe(true);
    expect(arr.length).toBe(3);
    expect(empty).toBe('');
    expect(acc).toBe(OK);
  });

  test('napi_create_string_utf8 replaces invalid UTF-8 with U+FFFD', () => {
    const points = (s: string) => [...s].map((c) => c.codePointAt(0));
    const [lone, cut2, cut3, surrogate, overlong, acc] = a.badUtf8();
    expect(points(lone)).toEqual([0x61, 0xfffd, 0x62]);
    expect(points(cut2)).toEqual([0xfffd]);
    // a truncated sequence is one replacement (its maximal subpart), the rest are per byte
    expect(points(cut3)).toEqual([0x78, 0xfffd]);
    expect(points(surrogate)).toEqual([0xfffd, 0xfffd, 0xfffd, 0x7a]);
    expect(points(overlong)).toEqual([0x1f600, 0xfffd, 0xfffd]);
    expect(acc).toBe(OK);
  });

  test('napi_typeof', () => {
    expect(a.typeOf(undefined)).toBe(0);
    expect(a.typeOf(null)).toBe(1);
    expect(a.typeOf(false)).toBe(2);
    expect(a.typeOf(3.5)).toBe(3);
    expect(a.typeOf('s')).toBe(4);
    expect(a.typeOf({})).toBe(6);
    expect(a.typeOf([1])).toBe(6);
    expect(a.typeOf(() => 1)).toBe(7);
    expect(a.typeOf(a.basics)).toBe(7);
    expect(a.status()).toBe(OK);
    // napi_symbol / napi_bigint (js_native_api_types.h); their wrapper objects are objects
    expect(a.typeOf(Symbol('s'))).toBe(5);
    expect(a.typeOf(BigInt(10))).toBe(9);
    expect(a.typeOf(Object(Symbol('s')))).toBe(6);
    expect(a.typeOf(Object(10n))).toBe(6);
    expect(a.status()).toBe(OK);
  });

  test('napi_get_value_string_utf8: length query, full copy, truncation, NULL result', () => {
    expect(a.strInfo('hello world')).toEqual([11, 'hello world', 'hel', 3, 'hello w', OK]);
    expect(a.strInfo('héllo')).toEqual([6, 'héllo', 'hé', 3, 'héllo', OK]);
    expect(a.strInfo('')).toEqual([0, '', '', 0, '', OK]);
  });

  test('napi_get_value_string_utf8 copies whole characters and never writes past bufsize', () => {
    const FILL = 0xaa; // untouched buffer byte
    // [status, copied, buffer bytes 0..=bufsize]
    expect(a.strCut('aé', 3)).toEqual([OK, 1, 97, 0, FILL, FILL]);
    expect(a.strCut('a€', 4)).toEqual([OK, 1, 97, 0, FILL, FILL, FILL]);
    expect(a.strCut('a€', 5)).toEqual([OK, 4, 97, 0xe2, 0x82, 0xac, 0, FILL]);
    expect(a.strCut('😀', 4)).toEqual([OK, 0, 0, FILL, FILL, FILL, FILL]);
    expect(a.strCut('😀', 5)).toEqual([OK, 4, 0xf0, 0x9f, 0x98, 0x80, 0, FILL]);
    // a lone surrogate is written as U+FFFD (and counted as its 3 bytes)
    expect(a.strCut('\ud800', 4)).toEqual([OK, 3, 0xef, 0xbf, 0xbd, 0, FILL]);
    expect(a.strStatus('a\ud800b')).toEqual([OK, 5]);
    // bufsize 0 with a buffer: nothing is written, 0 bytes copied
    expect(a.strCut('abc', 0)).toEqual([OK, 0, FILL]);
  });

  test('napi_get_value_string_utf8 of a non-string is napi_string_expected', () => {
    expect(a.strStatus('abc')).toEqual([OK, 3]);
    for (const v of [5, null, undefined, {}, Symbol('s'), new String('boxed')]) {
      expect(a.strStatus(v)).toEqual([STRING_EXPECTED, 999]);
    }
    // a NULL buffer is a length query, which needs `result`: without it, napi_invalid_arg
    expect(a.strNoOut('abc')).toBe(INVALID_ARG);
    expect(a.strNoOut(5)).toBe(STRING_EXPECTED);
  });

  test('napi_coerce_to_string', () => {
    expect(a.coerce(42)).toBe('42');
    expect(a.coerce(true)).toBe('true');
    expect(a.coerce({ toString: () => 'custom' })).toBe('custom');
    expect(a.status()).toBe(OK);
    expect(() =>
      a.coerce({
        toString() {
          throw new Error('toString threw');
        },
      }),
    ).toThrow('toString threw');
    expect(a.status()).toBe(STRING_EXPECTED);
    expect(() => a.coerce(Symbol('x'))).toThrow(TypeError);
    expect(a.status()).toBe(STRING_EXPECTED);
  });

  test('number / bool getters', () => {
    const NUM = NUMBER_EXPECTED;
    // [uint32, double, int64, bool, then the four statuses]
    expect(a.num(7.9)).toEqual([7, 7.9, 7, false, OK, OK, OK, BOOLEAN_EXPECTED]);
    expect(a.num(0)).toEqual([0, 0, 0, false, OK, OK, OK, BOOLEAN_EXPECTED]);
    expect(a.num(4294967295).slice(0, 3)).toEqual([4294967295, 4294967295, 4294967295]);
    // uint32 is ToUint32 (wraps modulo 2^32), as V8's Uint32Value
    expect(a.num(-1).slice(0, 3)).toEqual([4294967295, -1, -1]);
    expect(a.num(-3.5).slice(0, 3)).toEqual([4294967293, -3.5, -3]);
    expect(a.num(4294967296 + 5)[0]).toBe(5);
    expect(a.num(1e10).slice(0, 3)).toEqual([1410065408, 1e10, 1e10]);
    // non-finite numbers are 0 as uint32 and int64; int64 saturates otherwise
    expect(a.num(NaN).slice(0, 3)).toEqual([0, NaN, 0]);
    expect(a.num(Infinity).slice(0, 3)).toEqual([0, Infinity, 0]);
    expect(a.num(-Infinity).slice(0, 3)).toEqual([0, -Infinity, 0]);
    expect(a.num(2 ** 63)[2]).toBe(2 ** 63);
    expect(a.num(-(2 ** 64))[2]).toBe(-(2 ** 63));
    // no coercion: other types are napi_number_expected / napi_boolean_expected, outputs untouched
    for (const v of ['5', null, undefined, 10n, {}, new Number(1)]) {
      expect(a.num(v)).toEqual([99, 99, 99, false, NUM, NUM, NUM, BOOLEAN_EXPECTED]);
    }
    expect(a.num(true)).toEqual([99, 99, 99, true, NUM, NUM, NUM, OK]);
    expect(a.num(false)).toEqual([99, 99, 99, false, NUM, NUM, NUM, OK]);
  });
});

describe('objects and arrays', () => {
  test('named properties', () => {
    const o: Record<string, unknown> = {};
    expect(a.setNamed(o, 'k', 5)).toBe(OK);
    expect(o.k).toBe(5);
    expect(a.getNamed(o, 'k')).toBe(5);
    expect(a.getNamed(o, 'missing')).toBe(undefined);
    expect(a.status()).toBe(OK);
    // null / undefined: ToObject throws a TypeError and the call is napi_object_expected
    throwsToObjectError(() => a.setNamed(null, 'k', 1));
    expect(a.status()).toBe(OBJECT_EXPECTED);
    throwsToObjectError(() => a.getNamed(undefined, 'k'));
    expect(a.status()).toBe(OBJECT_EXPECTED);
    // other primitives are coerced to their wrapper objects
    expect(a.getNamed('abc', 'length')).toBe(3);
    expect(a.getNamed(5, 'toFixed')).toBe(Number.prototype.toFixed);
    expect(a.setNamed('str', 'k', 1)).toBe(OK);
    const trap = Object.defineProperty({}, 'boom', {
      get() {
        throw new Error('getter threw');
      },
    });
    expect(() => a.getNamed(trap, 'boom')).toThrow('getter threw');
    expect(a.status()).toBe(PENDING_EXCEPTION);
  });

  test('a set that throws reports napi_pending_exception', () => {
    const named = Object.defineProperty({}, 'boom', {
      set() {
        throw new Error('setter threw');
      },
    });
    expect(() => a.setNamed(named, 'boom', 1)).toThrow('setter threw');
    expect(a.status()).toBe(PENDING_EXCEPTION);
    const indexed = Object.defineProperty([], 0, {
      set() {
        throw new Error('index setter threw');
      },
    });
    expect(() => a.setEl(indexed, 0, 1)).toThrow('index setter threw');
    expect(a.status()).toBe(PENDING_EXCEPTION);
    // a write that silently fails (sloppy-mode Set) is not an error
    expect(a.setNamed(Object.freeze({ k: 1 }), 'k', 2)).toBe(OK);
    expect(a.setEl(Object.freeze([1]), 0, 2)).toBe(OK);
    expect(a.setNamed(new Proxy({}, { set: () => false }), 'k', 1)).toBe(OK);
  });

  test('elements, napi_is_array, napi_get_array_length', () => {
    const arr: unknown[] = [];
    expect(a.setEl(arr, 2, 'x')).toBe(OK);
    expect(arr.length).toBe(3);
    expect(arr[2]).toBe('x');
    expect(a.getEl(arr, 2)).toBe('x');
    expect(a.getEl(['a', 'b'], 5)).toBe(undefined);
    expect(a.status()).toBe(OK);
    throwsToObjectError(() => a.setEl(undefined, 0, 1));
    expect(a.status()).toBe(OBJECT_EXPECTED);
    throwsToObjectError(() => a.getEl(null, 0));
    expect(a.status()).toBe(OBJECT_EXPECTED);
    expect(a.getEl('xyz', 1)).toBe('y');
    expect(a.setEl(5, 0, 1)).toBe(OK);
    const trap = Object.defineProperty([], 0, {
      get() {
        throw new Error('index getter threw');
      },
    });
    expect(() => a.getEl(trap, 0)).toThrow('index getter threw');
    expect(a.status()).toBe(PENDING_EXCEPTION);

    expect(a.isArray([1])).toBe(true);
    expect(a.isArray({ length: 1 })).toBe(false);
    expect(a.arrLen([1, 2, 3])).toBe(3);
    expect(a.arrLen([])).toBe(0);
    expect(a.arrLen({ length: 2 })).toBe(undefined);
    expect(a.status()).toBe(ARRAY_EXPECTED);
    expect(a.arrLen('ab')).toBe(undefined);
    expect(a.status()).toBe(ARRAY_EXPECTED);
  });

  test('napi_get_property_names', () => {
    expect(a.keys({ x: 1, y: 2 })).toEqual(['x', 'y']);
    expect(a.status()).toBe(OK);
    // the for-in keys: enumerable string keys of the object and its prototype chain (no
    // symbols, no non-enumerables), integer indices converted to strings
    const proto = { inherited: 1 };
    Object.defineProperty(proto, 'hiddenInherited', { value: 1, enumerable: false });
    const o = Object.create(proto);
    o.own = 2;
    o[3] = 'x';
    o[Symbol('s')] = 1;
    Object.defineProperty(o, 'hidden', { value: 1, enumerable: false });
    expect(a.keys(o)).toEqual(['3', 'own', 'inherited']);
    expect(a.keys([5, 6])).toEqual(['0', '1']);
    expect(a.keys('ab')).toEqual(['0', '1']);
    expect(a.status()).toBe(OK);
    throwsToObjectError(() => a.keys(null));
    expect(a.status()).toBe(OBJECT_EXPECTED);
    const p = new Proxy(
      {},
      {
        ownKeys() {
          throw new Error('ownKeys threw');
        },
      },
    );
    expect(() => a.keys(p)).toThrow('ownKeys threw');
    expect(a.status()).toBe(PENDING_EXCEPTION);
  });
});

describe('functions', () => {
  test('napi_create_function + napi_get_cb_info (args, this, data)', () => {
    const f = a.makeFn(42);
    expect(a.status()).toBe(OK);
    expect(typeof f).toBe('function');
    const recv = { me: 1 };
    const r = f.call(recv, 'x', 2);
    expect(r[1]).toBe(recv);
    expect(r).toEqual([2, recv, 42, 'x', 2]);
    // more args than the addon's argv capacity (8): argc still reports the real count
    const many = f(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    expect(many[0]).toBe(10);
    expect(many.slice(3)).toEqual([1, 2, 3, 4]);
    expect(f()[0]).toBe(0);
    // each created function keeps its own data pointer
    expect(a.makeFn(7)()[2]).toBe(7);
    expect(f()[2]).toBe(42);
    // plain call: V8 hands API callbacks the global receiver
    expect(f()[1]).toBe(globalThis);
  });

  test('napi_create_function / napi_define_class name the function', () => {
    expect(a.makeFn(1).name).toBe('echo');
    const [auto, sized, unnamed, acc] = a.names();
    expect([auto.name, sized.name, unnamed.name]).toEqual(['echo', 'echo', '']);
    expect(acc).toBe(OK);
    expect(a.basics.name).toBe('basics');
    expect(a.Cls.name).toBe('Cls');
  });

  test('napi_get_cb_info with NULL argv / NULL argc', () => {
    expect(a.argcOnly(1, 2, 3)).toBe(3);
    expect(a.argcOnly()).toBe(0);
    const self = {};
    expect(a.thisOnly.call(self)).toBe(self);
    expect(a.status()).toBe(OK);
  });

  test('napi_get_cb_info fills argv slots past argc with undefined', () => {
    expect(a.padded(1)).toEqual([1, 1, undefined, undefined, undefined]);
    expect(a.padded()).toEqual([0, undefined, undefined, undefined, undefined]);
    expect(a.status()).toBe(OK);
    expect(a.padded(1, 2, 3, 4, 5)).toEqual([5, 1, 2, 3, 4]);
  });

  test('a callback returning NULL yields undefined', () => {
    expect(a.retNull(1)).toBe(undefined);
  });

  test('napi_define_class: the constructor runs with this + data', () => {
    const inst = new a.Cls('v1');
    expect(inst).toBeInstanceOf(a.Cls);
    expect(inst.value).toBe('v1');
    expect(inst.tag).toBe(7);
  });

  test('napi_call_function', () => {
    expect(a.call((x: number, y: number) => x + y, null, 2, 3)).toBe(5);
    expect(a.status()).toBe(OK);
    const obj = {
      v: 7,
      get(this: { v: number }) {
        return this.v;
      },
    };
    expect(a.call(obj.get, obj)).toBe(7);
    expect(a.call(() => 'no args', undefined)).toBe('no args');
    // a NULL recv is napi_invalid_arg and nothing is called
    let ran = false;
    expect(
      a.callNullRecv(() => {
        ran = true;
      }, 'p'),
    ).toBe(undefined);
    expect(a.status()).toBe(INVALID_ARG);
    expect(ran).toBe(false);
    expect(a.call(123, null)).toBe(undefined);
    expect(a.status()).toBe(INVALID_ARG);
    // argc > 0 with a NULL argv is napi_invalid_arg and nothing is called; argc 0 needs no argv
    let calls = 0;
    const count = () => {
      calls++;
    };
    expect(a.callNullArgv(count, 2)).toBe(INVALID_ARG);
    expect(calls).toBe(0);
    expect(a.callNullArgv(count, 0)).toBe(OK);
    expect(calls).toBe(1);
    expect(() =>
      a.call(() => {
        throw new RangeError('from js');
      }, null),
    ).toThrow(RangeError);
    expect(a.status()).toBe(PENDING_EXCEPTION);
  });
});

describe('arraybuffers, typed arrays, buffers', () => {
  test('napi_create_arraybuffer', () => {
    const ab = a.ab(4);
    expect(ab).toBeInstanceOf(ArrayBuffer);
    expect([...new Uint8Array(ab)]).toEqual([1, 2, 3, 4]);
    expect(a.ab(0).byteLength).toBe(0);
    expect(a.abNoData(3).byteLength).toBe(3);
    expect(a.status()).toBe(OK);
  });

  test('napi_create_typedarray: every type code', () => {
    const buf = new ArrayBuffer(32);
    const names = [
      'Int8Array',
      'Uint8Array',
      'Uint8ClampedArray',
      'Int16Array',
      'Uint16Array',
      'Int32Array',
      'Uint32Array',
      'Float32Array',
      'Float64Array',
      'BigInt64Array',
      'BigUint64Array',
      'Float16Array', // napi_float16_array (11), accepted by Node 26
    ];
    names.forEach((name, type) => {
      const ta = a.ta(type, 2, buf, 8);
      expect(a.status()).toBe(OK);
      expect(ta.constructor.name).toBe(name);
      expect(ta.length).toBe(2);
      expect(ta.byteOffset).toBe(8);
      expect(ta.buffer).toBe(buf);
    });
    // an unknown type code is napi_invalid_arg
    for (const type of [12, 99, -1]) {
      expect(a.ta(type, 4, buf, 0)).toBe(undefined);
      expect(a.status()).toBe(INVALID_ARG);
    }
    expect(a.ta(1, 1, {}, 0)).toBe(undefined);
    expect(a.status()).toBe(INVALID_ARG);
  });

  test('napi_create_typedarray: a view outside its buffer is a RangeError', () => {
    const buf = new ArrayBuffer(32);
    const rangeError = (f: () => unknown, message: string, code: string) => {
      let caught: any;
      try {
        f();
      } catch (e) {
        caught = e;
      }
      expect(caught).toBeInstanceOf(RangeError);
      expect([caught.message, caught.code]).toEqual([message, code]);
      expect(a.status()).toBe(GENERIC_FAILURE);
    };
    const ALIGNMENT = 'ERR_NAPI_INVALID_TYPEDARRAY_ALIGNMENT';
    const LENGTH = 'ERR_NAPI_INVALID_TYPEDARRAY_LENGTH';
    rangeError(
      () => a.ta(3, 1, buf, 1),
      'start offset of Int16Array should be a multiple of 2',
      ALIGNMENT,
    );
    rangeError(
      () => a.ta(10, 1, buf, 4),
      'start offset of BigUint64Array should be a multiple of 8',
      ALIGNMENT,
    );
    rangeError(() => a.ta(1, 33, buf, 0), 'Invalid typed array length', LENGTH);
    rangeError(() => a.ta(5, 7, buf, 8), 'Invalid typed array length', LENGTH);
    // exactly filling the buffer is fine
    expect(a.ta(5, 6, buf, 8).length).toBe(6);
    expect(a.status()).toBe(OK);
  });

  test('napi_create_typedarray: a length overflowing size_t is a RangeError, not an abort', () => {
    // (Node itself lets length * 8 wrap and dies in a V8 CHECK here)
    let caught: any;
    try {
      a.ta(8, 2 ** 61, new ArrayBuffer(8), 0);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RangeError);
    expect(caught.code).toBe('ERR_NAPI_INVALID_TYPEDARRAY_LENGTH');
    expect(a.status()).toBe(GENERIC_FAILURE);
  });

  test('napi_create_external_arraybuffer is over the external bytes', () => {
    expect([...new Uint8Array(a.extAb(3))]).toEqual([120, 121, 122]);
    expect(a.extAb(0).byteLength).toBe(0);
    expect(a.status()).toBe(OK);
  });

  test('napi_create_buffer / _copy / _external', () => {
    const b = a.buf(3);
    expect(b).toBeInstanceOf(Uint8Array);
    expect([...b]).toEqual([0, 2, 4]);
    expect(a.buf(0).length).toBe(0);
    expect(a.bufNoData(2).length).toBe(2);

    const [c, hadData] = a.bufCopy('hello', 5);
    expect(c).toBeInstanceOf(Uint8Array);
    // result_data aliases the new buffer: the addon poked 'Z' through it
    expect(String.fromCharCode(...c)).toBe('Zello');
    expect(hadData).toBe(true);
    const [c0, hadData0] = a.bufCopy('', 0);
    expect(c0.length).toBe(0);
    expect(hadData0).toBe(false);
    expect([...a.bufCopyNull(3)]).toEqual([0, 0, 0]);
    expect(String.fromCharCode(...a.bufCopyNoRes('abc'))).toBe('abc');

    expect(String.fromCharCode(...a.extBuf(4))).toBe('exte');
    expect(a.extBuf(0).length).toBe(0);
    expect([...a.extBufNull(2)]).toEqual([0, 0]);
    expect(a.status()).toBe(OK);
  });

  test('napi_get_buffer_info: views (with offset), non-buffers', () => {
    const u = new Uint8Array([9, 8, 7, 6]);
    expect(a.bufInfo(u.subarray(1))).toEqual([3, 8]);
    expect([...u]).toEqual([9, 9, 7, 6]); // data points at the view's first byte
    expect(a.bufInfo(new DataView(u.buffer, 2, 2))).toEqual([2, 7]);
    expect(u[2]).toBe(8);
    expect(a.bufInfo(new Uint8Array(0))).toEqual([0, -1]);
    expect(a.status()).toBe(OK);
    // only ArrayBufferViews (Buffer / Uint8Array / DataView ...): a raw ArrayBuffer is not one
    for (const v of ['nope', new Uint8Array([5]).buffer, new ArrayBuffer(0)]) {
      expect(a.bufInfo(v)).toBe(undefined);
      expect(a.status()).toBe(INVALID_ARG);
    }
    expect(a.bufInfoNulls(u)).toBe(OK);
    expect(a.bufInfoNulls(new ArrayBuffer(1))).toBe(INVALID_ARG);
  });
});

describe('napi_new_instance', () => {
  test('constructs an instance of a class', () => {
    class C {
      made = true;
    }
    const o = a.newInstance(C);
    expect(o).toBeInstanceOf(C);
    expect(o.made).toBe(true);
    expect(a.status()).toBe(OK);
  });
});

describe('errors and exceptions', () => {
  test('napi_throw passes the value through unchanged', () => {
    let caught: unknown = 'nothing';
    try {
      a.throwVal(42);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBe(42);
    expect(a.status()).toBe(OK);
    const err = new TypeError('mine');
    expect(() => a.throwVal(err)).toThrow(err);
  });

  test('napi_throw_error throws an Error with the message', () => {
    let caught: any;
    try {
      a.throwErr('native boom');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    expect(caught.message).toBe('native boom');
    expect(a.status()).toBe(OK);
  });

  test('napi_throw_error / napi_create_error set err.code', () => {
    let caught: any;
    try {
      a.throwErrCode('ERR_NATIVE', 'with code');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    expect(caught.message).toBe('with code');
    expect(caught.code).toBe('ERR_NATIVE');
    expect(a.status()).toBe(OK);
    try {
      a.throwErr('no code');
    } catch (e) {
      caught = e;
    }
    expect('code' in caught).toBe(false); // a NULL code sets none
    const made = a.createErrCode('ERR_MADE', 'made');
    expect(a.status()).toBe(OK);
    expect(made).toBeInstanceOf(Error);
    expect([made.message, made.code]).toEqual(['made', 'ERR_MADE']);
    expect('code' in a.createErr('plain')).toBe(false);
    // a code value must be a string
    for (const code of [5, undefined, null, new String('boxed')]) {
      expect(a.createErrCode(code, 'm')).toBe(undefined);
      expect(a.status()).toBe(STRING_EXPECTED);
    }
  });

  test('a code that cannot be set is napi_generic_failure', () => {
    Object.defineProperty(Error.prototype, 'code', {
      set() {
        throw new Error('code setter threw');
      },
      configurable: true,
    });
    try {
      expect(() => a.createErrCode('ERR_X', 'm')).toThrow('code setter threw');
      expect(a.status()).toBe(GENERIC_FAILURE);
      expect(() => a.throwErrCode('ERR_X', 'm')).toThrow('code setter threw');
      expect(a.status()).toBe(GENERIC_FAILURE);
    } finally {
      delete (Error.prototype as any).code;
    }
  });

  test('napi_create_error / napi_is_error', () => {
    const e = a.createErr('made');
    expect(a.status()).toBe(OK);
    expect(e).toBeInstanceOf(Error);
    expect(e.message).toBe('made');
    expect(a.createErr(5)).toBe(undefined);
    expect(a.status()).toBe(STRING_EXPECTED);
    expect(a.createErr(new String('boxed'))).toBe(undefined);
    expect(a.status()).toBe(STRING_EXPECTED);
    expect(a.isError(new Error('x'))).toBe(true);
    expect(a.isError(new RangeError('x'))).toBe(true);
    expect(a.isError(e)).toBe(true);
    expect(a.isError({ message: 'x' })).toBe(false);
    expect(a.isError('x')).toBe(false);
  });

  test('no exception pending outside an error', () => {
    expect(a.pending()).toEqual([false, undefined, OK]);
  });

  test('napi_throw_error leaves the exception pending until it is cleared', () => {
    const [st, before, ex, after, acc] = a.throwPending('parked');
    expect(st).toBe(OK);
    expect(before).toBe(true);
    expect(ex).toBeInstanceOf(Error);
    expect(ex.message).toBe('parked');
    expect(after).toBe(false);
    expect(acc).toBe(OK);
  });

  test('a napi_call_function that threw leaves its exception pending', () => {
    const [st, pending, ex, noResult, acc] = a.callPending(() => {
      throw new TypeError('from the callee');
    });
    expect(st).toBe(PENDING_EXCEPTION);
    expect(pending).toBe(true);
    expect(ex).toBeInstanceOf(TypeError);
    expect(ex.message).toBe('from the callee');
    expect(noResult).toBe(true);
    expect(acc).toBe(OK);
    expect(a.callPending(() => 1)).toEqual([OK, false, undefined, false, OK]);
  });

  test('with an exception pending, calls that can run JS return napi_pending_exception', () => {
    let ran = false;
    const r = a.afterThrow(
      () => {
        ran = true;
      },
      class {},
    );
    const ex = r.pop();
    expect(r.slice(0, 22)).toEqual(Array(22).fill(PENDING_EXCEPTION));
    expect(r.slice(22)).toEqual(Array(23).fill(OK));
    expect(ran).toBe(false);
    // the first exception is the pending one; later throws did not replace it
    expect(ex.message).toBe('first');
  });
});

describe('references', () => {
  test('create / get / unref / delete', () => {
    const target = { id: 1 };
    const id = a.ref(target);
    expect(a.status()).toBe(OK);
    expect(id).toBeGreaterThan(0);
    expect(a.deref(id)).toBe(target);
    const second = { id: 2 };
    const id2 = a.ref(second);
    expect(id2).not.toBe(id);
    expect(a.deref(id2)).toBe(second);
    // as Node (module API version 8): a primitive cannot be referenced
    expect(a.ref('third')).toBe(0);
    expect(a.status()).toBe(INVALID_ARG);
    expect(a.refUnref(id)).toBe(OK);
    expect(a.deref(id)).toBe(target);
    expect(a.delRef(id)).toBe(OK);
    expect(a.deref(id)).toBe(undefined);
    expect(a.status()).toBe(OK);
    expect(a.deref(id2)).toBe(second);
  });

  test('unknown references', () => {
    expect(a.deref(0)).toBe(undefined);
    expect(a.status()).toBe(INVALID_ARG);
    expect(a.deref(1e6)).toBe(undefined);
    expect(a.status()).toBe(INVALID_ARG);
    expect(a.delRef(1e6)).toBe(OK);
  });

  test('napi_delete_reference of a NULL reference is napi_invalid_arg', () => {
    expect(a.delRef(0)).toBe(INVALID_ARG);
  });
});

describe('stubs', () => {
  test('cleanup hooks, napi_stub, threadsafe functions', () => {
    // a NULL cleanup hook, a threadsafe function without a name, calls on the NULL handle
    // it left: napi_invalid_arg (Node aborts on a NULL threadsafe function)
    expect(a.stubs()).toEqual([INVALID_ARG, GENERIC_FAILURE, INVALID_ARG, false, INVALID_ARG,
      INVALID_ARG]);
  });
});
