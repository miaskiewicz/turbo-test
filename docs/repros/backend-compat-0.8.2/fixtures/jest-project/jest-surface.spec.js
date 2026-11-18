jest.mock('./mock-target.cjs', () => ({ calculate: jest.fn(() => 11) }));
jest.mock('fixture-virtual', () => ({ value: 'virtual' }), { virtual: true });

test('loads a factory mock and its unmocked implementation', () => {
  expect(require('./mock-target.cjs').calculate()).toBe(11);
  expect(jest.requireActual('./mock-target.cjs').calculate()).toBe(7);
});

test('loads a virtual mock', () => {
  expect(require('fixture-virtual').value).toBe('virtual');
});

test('resetAllMocks clears a Jest mock implementation', () => {
  const mock = jest.fn(() => 42);
  expect(mock()).toBe(42);
  jest.resetAllMocks();
  expect(mock()).toBeUndefined();
});

test('restoreAllMocks restores a regular spy', () => {
  const object = { value: () => 42 };
  jest.spyOn(object, 'value').mockReturnValue(9);
  expect(object.value()).toBe(9);
  jest.restoreAllMocks();
  expect(object.value()).toBe(42);
});

test('advanced fake timers execute scheduled callbacks', () => {
  jest.useFakeTimers({ now: 1000 });
  try {
    const callback = jest.fn();
    setTimeout(callback, 20);
    jest.advanceTimersByTime(20);
    expect(callback).toHaveBeenCalledTimes(1);
    expect(Date.now()).toBe(1020);
  } finally {
    jest.useRealTimers();
  }
});

test('resetModules clears cached local modules', () => {
  const original = require('./module-state.cjs');
  jest.resetModules();
  expect(require('./module-state.cjs')).not.toBe(original);
});

test('requireActual propagates a missing-module error', () => {
  expect(() => jest.requireActual('./intentionally-missing.cjs')).toThrow(/Cannot find module/);
});

test('requireMock generates a mock for an unmocked module', () => {
  const mock = jest.requireMock('./unmocked-target.cjs');
  expect(jest.isMockFunction(mock.calculate)).toBe(true);
  expect(mock.calculate()).toBeUndefined();
});

test('runAllTicks flushes process.nextTick with modern fake timers', () => {
  jest.useFakeTimers();
  try {
    const callback = jest.fn();
    process.nextTick(callback);
    jest.runAllTicks();
    expect(callback).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
  }
});

test('local automock creates mock functions', () => {
  jest.resetModules();
  jest.enableAutomock();
  try {
    expect(jest.isMockFunction(require('./unmocked-target.cjs').calculate)).toBe(true);
  } finally {
    jest.disableAutomock();
    jest.resetModules();
  }
});
