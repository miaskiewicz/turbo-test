const testCase=globalThis.test||require('node:test');
const assert=require('node:assert/strict');
const crypto=require('node:crypto');
testCase('HMAC known SHA1 and SHA256 answers',()=>{
 assert.equal(crypto.createHmac('sha1',Buffer.alloc(20,11)).update('Hi There').digest('hex'),'b617318655057264e28bc0b6fb378c8ef146be00');
 assert.equal(crypto.createHmac('sha256',Buffer.alloc(20,11)).update('Hi There').digest('hex'),'b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7');
});
testCase('HMAC key views copies incremental encoding and lifecycle',()=>{
 const key=Buffer.from('!key!');const h=crypto.createHmac('SHA256',key.subarray(1,4));key.fill(0);
 h.update('The quick brown ').update('fox jumps over the lazy dog');
 assert.equal(h.digest('hex'),'f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8');
 assert.throws(()=>h.update('late'));assert.equal(h.digest().length,0);
 assert.notDeepEqual(crypto.createHmac('sha1','a').update('b').digest(),crypto.createHmac('sha1','b').update('b').digest());
});
testCase('PBKDF2 SHA1 RFC6070 answers and binary inputs',()=>{
 assert.equal(crypto.pbkdf2Sync('password','salt',1,20,'sha1').toString('hex'),'0c60c80f961f0e71f3a9b524af6012062fe037a6');
 assert.equal(crypto.pbkdf2Sync(Buffer.from('password'),Buffer.from('salt'),2,20,'sha1').toString('hex'),'ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957');
 assert.equal(crypto.pbkdf2Sync('password','salt',4096,25,'sha1').toString('hex'),'4b007901b765489abead49d926f721d065a429c12e463f6c4cd');
});
testCase('PBKDF2 validates iteration and output length',()=>{
 assert.throws(()=>crypto.pbkdf2Sync('p','s',0,20,'sha1'));
 assert.throws(()=>crypto.pbkdf2Sync('p','s',1,-1,'sha1'));
 assert.throws(()=>crypto.pbkdf2Sync('p','s',1.5,20,'sha1'));
 assert.equal(crypto.pbkdf2Sync('p','s',1,0,'sha1').length,0);
});
