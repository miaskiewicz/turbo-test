test('supports generating an EC key pair', () => {
  const { generateKeyPairSync } = require('node:crypto');
  expect(generateKeyPairSync('ec', { namedCurve: 'prime256v1' }).publicKey.type).toBe('public');
});
