import { describe, test, expect } from 'vitest';

// objectwrap.node (tests/napi_support/objectwrap.c) is compiled by tests/napi_host.rs next to
// this file: a node-addon-api-style ObjectWrap class defined with napi_define_class.
const w = require('./objectwrap.node');
const { Point } = w;

const OK = 0;
const INVALID_ARG = 1;
const NAME_EXPECTED = 4;
const GENERIC_FAILURE = 9;
const PENDING_EXCEPTION = 10;

// [has a value (vs get/set), writable, enumerable, configurable] of an own property
function shape(o: object, k: PropertyKey) {
  const d = Object.getOwnPropertyDescriptor(o, k)!;
  return 'value' in d
    ? ['value', d.writable, d.enumerable, d.configurable]
    : [typeof d.get, typeof d.set, d.enumerable, d.configurable];
}

describe('napi_define_class', () => {
  test('the constructor: a class named by the descriptor, new wraps a native object', () => {
    expect(typeof Point).toBe('function');
    expect(Point.name).toBe('Point');
    const p = new Point(3, 4);
    expect(p).toBeInstanceOf(Point);
    // (compared outside expect(): the host's matchers format their operands eagerly, and the
    // prototype's members throw for a receiver that is not a Point)
    expect(Object.getPrototypeOf(p) === Point.prototype).toBe(true);
    expect(Reflect.ownKeys(p)).toEqual([]);
    // ObjectWrap's constructor: napi_get_new_target is NULL for a plain call
    expect(() => Point(1, 2)).toThrow("Class constructors cannot be invoked without 'new'");
    class Sub extends Point {}
    const s = new Sub(1, 1);
    expect(s).toBeInstanceOf(Point);
    expect(s.norm2()).toBe(2);
  });

  test('instance methods live on the prototype with their attributes and data', () => {
    const p = new Point(3, 4);
    expect(p.norm2()).toBe(25);
    p.scale(2);
    expect(p.norm2()).toBe(100);
    expect(+p).toBe(14);
    const proto = Point.prototype;
    // (V8's template order: the string keys, `constructor`, then the symbols)
    expect(Reflect.ownKeys(proto)).toEqual([
      'norm2',
      'scale',
      'x',
      'y',
      'both',
      'kind',
      'constructor',
      Symbol.toPrimitive,
    ]);
    // napi_default: read-only, not enumerable, not configurable; napi_default_method: writable
    // and configurable
    expect(shape(proto, 'norm2')).toEqual(['value', false, false, false]);
    expect(shape(proto, 'scale')).toEqual(['value', true, false, true]);
    expect(shape(proto, Symbol.toPrimitive)).toEqual(['value', false, false, false]);
    expect(typeof proto.norm2).toBe('function');
    expect(proto.norm2.name).toBe('norm2');
    // a method has the class as its signature: another receiver is an illegal invocation
    expect(() => proto.norm2.call({})).toThrow(TypeError);
    expect(() => proto.norm2.call({})).toThrow('Illegal invocation');
    // an instance of the class that the native constructor did not wrap reaches napi_unwrap
    expect(() => proto.norm2.call(Object.create(proto))).toThrow(TypeError);
  });

  test('instance accessors and values', () => {
    const p = new Point(3, 4);
    const proto = Point.prototype;
    expect(shape(proto, 'x')).toEqual(['function', 'function', true, false]);
    expect(shape(proto, 'y')).toEqual(['function', 'undefined', false, true]);
    expect(shape(proto, 'both')).toEqual(['undefined', 'function', false, false]);
    expect([p.x, p.y]).toEqual([3, 4]);
    p.x = 10;
    expect([p.x, p.y]).toEqual([10, 4]);
    // a getter-only accessor ignores a sloppy-mode set (and throws in strict mode)
    expect(() => {
      'use strict';
      p.y = 1;
    }).toThrow(TypeError);
    expect(p.y).toBe(4);
    p.both = 7;
    expect([p.x, p.y, p.both]).toEqual([7, 7, undefined]);
    // an accessor of an object the constructor did not wrap
    expect(() => Object.getOwnPropertyDescriptor(proto, 'x')!.get!.call({})).toThrow('not a Point');
    expect(p.kind).toBe('point');
    expect(shape(proto, 'kind')).toEqual(['value', false, false, false]);
    expect(Object.keys(p)).toEqual([]);
    const keys: string[] = [];
    for (const k in p) keys.push(k);
    expect(keys).toEqual(['x']);
  });

  test('static members are own properties of the constructor', () => {
    expect(shape(Point, 'created')).toEqual(['value', true, false, true]);
    expect(shape(Point, 'tag')).toEqual(['value', false, true, false]);
    expect(shape(Point, 'live')).toEqual(['function', 'undefined', false, false]);
    expect(shape(Point, 'precision')).toEqual(['function', 'function', false, true]);
    expect(shape(Point, 'dimensions')).toEqual(['value', false, true, false]);
    const before = Point.created();
    new Point(0, 0);
    expect(Point.created()).toBe(before + 1);
    expect(Point.created.name).toBe('created');
    // a C static method gets the descriptor's data; a static method has no signature
    expect(Point.tag()).toBe('static data');
    expect(Point.tag.call(undefined)).toBe('static data');
    expect(Point.tag.name).toBe('');
    expect(Point.live).toBeGreaterThan(0);
    expect(Point.precision).toBe(2);
    Point.precision = 5;
    expect(Point.precision).toBe(5);
    expect(Point.dimensions).toBe(2);
    expect(Object.keys(Point)).toEqual(['tag', 'dimensions']);
    // static members are not on the prototype or instances
    expect('created' in Point.prototype).toBe(false);
    expect((new Point(0, 0) as any).dimensions).toBe(undefined);
  });

  test('argument checks', () => {
    const [st, partial] = w.defineClassChecks();
    // NULL result, NULL constructor, properties missing for a count, NULL name, a non-name
    // napi_value name, the same as a static (the class is returned all the same), pending
    // exception
    expect(st).toEqual([
      INVALID_ARG,
      INVALID_ARG,
      INVALID_ARG,
      INVALID_ARG,
      NAME_EXPECTED,
      NAME_EXPECTED,
      PENDING_EXCEPTION,
    ]);
    expect(typeof partial).toBe('function');
    // the name's length is honoured
    expect(partial.name).toBe('Partial');
  });

  test('napi_get_new_target needs its arguments', () => {
    expect(w.newTargetChecks()).toEqual([INVALID_ARG, INVALID_ARG]);
  });

  // host-only: Node aborts (an object in a template) or crashes (NULL values) on these
  test('descriptors Node aborts on are refused', () => {
    // an object InstanceValue, a NULL InstanceValue, no name, a NULL napi_define_properties value
    expect(w.hostOnlyChecks()).toEqual([INVALID_ARG, INVALID_ARG, NAME_EXPECTED, INVALID_ARG]);
  });
});

describe('napi_define_properties', () => {
  test('data properties, a method and an accessor', () => {
    const o: any = {};
    expect(w.defineProperties(o)).toBe(OK);
    expect(Reflect.ownKeys(o)).toEqual(['a', 'b', 'm', 'p']);
    expect(shape(o, 'a')).toEqual(['value', true, true, true]);
    expect(shape(o, 'b')).toEqual(['value', false, true, false]);
    expect(shape(o, 'm')).toEqual(['value', true, false, false]);
    expect(shape(o, 'p')).toEqual(['function', 'function', true, false]);
    expect([o.a, o.b, o.m(), o.m.name]).toEqual([1, 2, 'method data', '']);
    o.p = 9;
    expect(o.p).toBe(9);
    // a primitive target is converted with ToObject (the definitions land on a temporary)
    expect(w.defineProperties(5)).toBe(OK);
  });

  test('argument checks and failed definitions', () => {
    // NULL object, properties missing for a count, a non-name name; on a frozen object: a
    // data property (DefineProperty and the CreateDataProperty fast path), a method, an accessor
    expect(w.definePropertiesChecks(Object.freeze({}))).toEqual([
      INVALID_ARG,
      INVALID_ARG,
      NAME_EXPECTED,
      INVALID_ARG,
      INVALID_ARG,
      GENERIC_FAILURE,
      INVALID_ARG,
    ]);
    // ToObject of undefined throws: napi_object_expected, and the TypeError reaches the caller
    expect(() => w.defineProperties(undefined)).toThrow(TypeError);
  });

  test('an accessor with one half leaves the other half of an existing accessor alone', () => {
    const get = () => 1;
    const set = (_v: unknown) => {};
    for (const setterOnly of [false, true]) {
      const o: any = {};
      Object.defineProperty(o, 'p', { get, set, configurable: true });
      expect(w.defineAccessorHalf(o, setterOnly)).toBe(OK);
      const d = Object.getOwnPropertyDescriptor(o, 'p')!;
      // the half given is a new function, the other half is the existing one
      expect(setterOnly ? d.get === get : d.set === set).toBe(true);
      expect(setterOnly ? d.set === set : d.get === get).toBe(false);
      expect(typeof (setterOnly ? d.set : d.get)).toBe('function');
      expect([d.enumerable, d.configurable]).toEqual([false, true]);
      // on a new property the missing half is undefined
      const n: any = {};
      expect(w.defineAccessorHalf(n, setterOnly)).toBe(OK);
      const nd = Object.getOwnPropertyDescriptor(n, 'p')!;
      expect(setterOnly ? nd.get : nd.set).toBe(undefined);
    }
  });
});
