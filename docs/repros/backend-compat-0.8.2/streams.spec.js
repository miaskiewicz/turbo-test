test('loads the promises stream API', () => {
  expect(typeof require('stream/promises').pipeline).toBe('function');
});
