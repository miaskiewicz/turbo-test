import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const source=fs.readFileSync(new URL('../src/node_buffer_byte_length.js',import.meta.url),'utf8');
test('shared buffer byte length survives constructor installation after framework snapshot',()=>{
 const context=vm.createContext({SharedArrayBuffer:undefined});
 vm.runInContext(source+'\nglobalThis.byteLength=__bufferByteLength;',context);
 context.SharedArrayBuffer=SharedArrayBuffer;
 assert.equal(context.byteLength(new SharedArrayBuffer(11)),11);
 assert.throws(()=>context.byteLength({byteLength:11}),{code:'ERR_INVALID_ARG_TYPE'});
 context.SharedArrayBuffer=undefined;
 assert.equal(context.byteLength(new SharedArrayBuffer(7)),7);
});
