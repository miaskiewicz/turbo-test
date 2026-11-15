import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
const read=(name)=>fs.readFileSync(new URL('../src/'+name,import.meta.url),'utf8');
test('shared startup source physically excludes backend payload markers and reset calls',()=>{
 const shared=read('runtime.js');
 for(const token of ['__TT_NODE_KEYS__','__TT_NODE_AES__','__TT_NODE_X509__','__TT_NODE_HTTP__','__TT_NODE_TCP__','__TT_NODE_VM__','__TT_NODE_PROCESSES__','__TT_NODE_ASYNC_PROCESSES__','__TT_NODE_ASYNC_CONTEXT__','__TT_NODE_ASYNC_HOOKS__'])assert.equal(shared.includes(token),false,token);
 for(const token of ['__httpReset();','__tcpReset();','__asyncProcessReset();','__resetAsyncHooks();'])assert.equal(shared.includes(token),false,token);
});
test('backend payload owns native capabilities while frontend HMAC remains shared',()=>{
 const backend=read('runtime_backend.js');
 for(const token of ['__TT_NODE_KEYS__','__TT_NODE_AES__','__TT_NODE_HTTP__','__TT_NODE_TCP__','__TT_NODE_VM__','__TT_NODE_ASYNC_PROCESSES__'])assert.equal(backend.includes(token),true,token);
 assert.equal(read('node_crypto_hashes.js').includes('function __cryptoCreateHmac'),true);
 assert.equal(read('node_crypto_hashes.js').includes('function __cryptoPbkdf2'),false);
});
