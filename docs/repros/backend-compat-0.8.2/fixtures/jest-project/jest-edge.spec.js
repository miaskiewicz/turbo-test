jest.mock('./edge-manual.cjs');

test('uses a colocated manual mock for a factory-less jest.mock', () => {
  expect(require('./edge-manual.cjs').load()).toBe('manual');
});

test('passes the module path and mock to onGenerateMock callbacks in order', () => {
  const seen = [];
  jest.onGenerateMock((path, mock) => {
    seen.push(path.endsWith('edge-generated.cjs'));
    return { ...mock, first: true };
  });
  jest.onGenerateMock((_path, mock) => ({ ...mock, second: true }));
  jest.mock('./edge-generated.cjs');
  const generated = require('./edge-generated.cjs');
  expect(seen).toEqual([true]);
  expect([generated.first, generated.second]).toEqual([true, true]);
});

test('keeps a numeric JEST_WORKER_ID in the suite environment', () => {
  expect(process.env.JEST_WORKER_ID).toMatch(/^[1-9]\d*$/);
});

test('replaces a property again through the replacement handle', () => {
  const obj = { value: 1 };
  const handle = jest.replaceProperty(obj, 'value', 2);
  try {
    handle.replaceValue(3);
    expect(obj.value).toBe(3);
  } finally {
    handle.restore();
  }
  expect(obj.value).toBe(1);
});

test('temporarily installs an async mock implementation', async () => {
  const mock = jest.fn(() => 'original');
  await mock.withImplementation(() => 'temporary', async () => {
    await Promise.resolve();
    expect(mock()).toBe('temporary');
  });
  expect(mock()).toBe('original');
});

test('preserves relative timer delays when the wall clock jumps', () => {
  jest.useFakeTimers({ now: 1000 });
  try {
    const callback = jest.fn();
    setTimeout(callback, 100);
    jest.setSystemTime(10000);
    expect(callback).not.toHaveBeenCalled();
    jest.advanceTimersByTime(99);
    expect(callback).not.toHaveBeenCalled();
    jest.advanceTimersByTime(1);
    expect(callback).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
  }
});

test('does not drain newly scheduled timers in runOnlyPendingTimers', () => {
  jest.useFakeTimers();
  try {
    const nested = jest.fn();
    setTimeout(() => setTimeout(nested, 10), 10);
    jest.runOnlyPendingTimers();
    expect(nested).not.toHaveBeenCalled();
    jest.runOnlyPendingTimers();
    expect(nested).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
  }
});
