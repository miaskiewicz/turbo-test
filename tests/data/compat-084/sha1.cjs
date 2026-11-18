const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
testCase('SHA1 known answers and encoding', () => {
 assert.equal(createHash('sha1').update('abc').digest('hex'), 'a9993e364706816aba3e25717850c26c9cd0d89d');
 assert.equal(createHash('SHA1').digest('hex'), 'da39a3ee5e6b4b0d3255bfef95601890afd80709');
 assert.equal(createHash('sha1').update('abc').digest('base64'), 'qZk+NkcGgWq6PiVxeFDCbJzQ2J0=');
});
testCase('SHA1 incremental Unicode and binary views', () => {
 const text = 'hello 🌍';
 const expected = createHash('sha1').update(text).digest('hex');
 const bytes = Buffer.from(text);
 assert.equal(createHash('sha1').update(bytes.subarray(0, 4)).update(bytes.subarray(4)).digest('hex'), expected);
 const view = new Uint8Array([0,97,98,99,0]);
 const hash = createHash('sha1').update(view.subarray(1,4)); view.fill(0);
 assert.equal(hash.digest('hex'), 'a9993e364706816aba3e25717850c26c9cd0d89d');
});
testCase('SHA1 default Buffer and lifecycle', () => {
 const hash = createHash('sha1').update('abc');
 assert.equal(Buffer.isBuffer(hash.digest()), true);
 assert.throws(() => hash.update('x'));
 assert.throws(() => hash.digest());
});
testCase('SHA256 remains correct', () => {
 assert.equal(createHash('sha256').update('abc').digest('hex'), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
});
