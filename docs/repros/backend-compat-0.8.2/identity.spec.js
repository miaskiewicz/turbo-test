test('re-export shares identity with the direct entry', () => {
  expect(require('identity-fixture').state).toBe(require('identity-fixture/state').state);
});
