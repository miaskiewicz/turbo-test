// crypto.createCipheriv / createDecipheriv argument validation as Node v26.11.0 (#450): types,
// codes, messages and check order (cipher type, key type, iv type, then cipher, key length, iv,
// auth tag). Also passes under node --test.
const test = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');

const key = Buffer.alloc(32);
const iv = Buffer.alloc(12);
const KEY_TYPES = 'string or an instance of ArrayBuffer, Buffer, TypedArray, DataView, KeyObject, or CryptoKey';
const IV_TYPES = 'string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView';
const argType = (name, types, received) => ({ name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE', message: `The "${name}" argument must be of type ${types}. Received ${received}` });
const invalidIv = { name: 'TypeError', code: 'ERR_CRYPTO_INVALID_IV', message: 'Invalid initialization vector' };

for (const create of ['createCipheriv', 'createDecipheriv']) {
  const make = (...args) => () => crypto[create](...args);
  test(`${create} validates argument types in Node's order`, () => {
    assert.throws(make(), argType('cipher', 'string', 'undefined'));
    assert.throws(make(5, key, iv), argType('cipher', 'string', 'type number (5)'));
    assert.throws(make('aes-256-gcm'), argType('key', KEY_TYPES, 'undefined'));
    assert.throws(make('nope', 5, iv), argType('key', KEY_TYPES, 'type number (5)'));
    assert.throws(make('aes-256-gcm', key), argType('iv', IV_TYPES, 'undefined'));
    assert.throws(make('aes-256-gcm', key, 5), argType('iv', IV_TYPES, 'type number (5)'));
  });
  test(`${create} rejects bad values like Node`, () => {
    assert.throws(make('nope-128', key, iv), { code: 'ERR_CRYPTO_UNKNOWN_CIPHER' });
    assert.throws(make('aes-256-gcm', Buffer.alloc(16), iv), { name: 'RangeError', code: 'ERR_CRYPTO_INVALID_KEYLEN', message: 'Invalid key length' });
    assert.throws(make('aes-256-gcm', key, Buffer.alloc(0)), invalidIv);
    assert.throws(make('aes-256-gcm', key, null), invalidIv);
    assert.throws(make('aes-256-gcm', key, iv, { authTagLength: 17 }), { name: 'TypeError', code: 'ERR_CRYPTO_INVALID_AUTH_TAG', message: 'Invalid authentication tag length: 17' });
    assert.throws(make('aes-256-gcm', key, iv, { authTagLength: '16' }), { name: 'TypeError', code: 'ERR_INVALID_ARG_VALUE', message: "The property 'options.authTagLength' is invalid. Received '16'" });
  });
  test(`${create} accepts a string key and an upper-case cipher name`, () => {
    assert.equal(typeof make('AES-256-GCM', 'k'.repeat(32), iv)().update, 'function');
  });
}
