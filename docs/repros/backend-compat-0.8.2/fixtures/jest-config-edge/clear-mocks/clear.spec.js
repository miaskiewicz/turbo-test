const mock = jest.fn(() => 42);

test('calls the mock once', () => {
  expect(mock()).toBe(42);
  expect(mock).toHaveBeenCalledTimes(1);
});

test('clears call history before the next test without clearing implementation', () => {
  expect(mock).not.toHaveBeenCalled();
  expect(mock()).toBe(42);
});
