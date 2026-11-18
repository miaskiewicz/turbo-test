const testCase=globalThis.test;
const assert=require('node:assert/strict');
const crypto=require('node:crypto');

testCase('PBKDF2 rejects unsupported total derivation work',()=>{
 // The previous implementation does only about one million HMACs here: safe RED,
 // unlike testing INT_MAX iterations through the synchronous native callback.
 assert.throws(()=>crypto.pbkdf2Sync('p','s',20,1048576,'sha1'),/work limit/);
});
