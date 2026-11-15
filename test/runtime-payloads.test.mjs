import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import {createRequire} from 'node:module';
import crypto from 'node:crypto';
import {constants as bufferLimits} from 'node:buffer';
import {pathToFileURL} from 'node:url';
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

function assembled(name) {
 const fragments=new Map([...read('runtime_sources.rs').matchAll(/"(\/\* __TT_[A-Z0-9_]+__ \*\/)",\s*include_str!\("([^"]+)"\)/g)].map(match=>[match[1],read(match[2])]));
 // These controls run in Node's VM; native tests separately check the linked V8 limits.
 fragments.set('/* __TT_NODE_BUFFER_LIMITS__ */',`MAX_LENGTH: ${bufferLimits.MAX_LENGTH}, MAX_STRING_LENGTH: ${bufferLimits.MAX_STRING_LENGTH}`);
 let source=read(name);
 for(const [marker,fragment] of fragments) source=source.replaceAll(marker,()=>fragment);
 assert.equal(source.includes('/* __TT_'),false);
 return source;
}
const require=createRequire(import.meta.url);
const chai=await import(pathToFileURL(process.env.TURBO_CHAI || require.resolve('chai')).href);
function commonContext() {
 const context=vm.createContext({TextEncoder,TextDecoder,__ttLoadChai:()=>chai,log(){},__ttWrite(){},
  __ttRandomBytes:(length)=>new Uint8Array(crypto.randomBytes(length)),
  __ttCryptoHashes:(operation,key,data)=>new Uint8Array(crypto.createHmac(operation.slice(5),key).update(data).digest()).buffer,
 });
 vm.runInContext(assembled('runtime.js'),context,{timeout:3000});
 return context;
}
test('shared initialization creates no backend implementations or DOM-only globals',()=>{
 const context=commonContext();
 assert.equal(context.__ttRuntimeBackendInstalled,false);
 for(const name of ['__nodeKeys','__nodeAes','__vmContexts','__tcpSockets','__httpConnections','__asyncChildren','__enabledAsyncHooks'])assert.equal(vm.runInContext('typeof '+name,context),'undefined',name);
 for(const name of ['DOMPoint','DOMMatrix','FileReader','MutationObserver','localStorage'])assert.equal(typeof context[name],'undefined',name);
 assert.throws(()=>vm.runInContext('__nodeBuiltins.net',context),{code:'ERR_TURBO_BACKEND_CAPABILITY'});
 assert.throws(()=>vm.runInContext("__nodeBuiltins.crypto.createCipheriv()",context),{code:'ERR_TURBO_BACKEND_CAPABILITY'});
 assert.equal(vm.runInContext("__nodeBuiltins.crypto.createHmac('sha256','key').update('message').digest('hex')",context),crypto.createHmac('sha256','key').update('message').digest('hex'));
 assert.equal(vm.runInContext('Object.values(__nodeBuiltins.crypto).length>0',context),true);
 assert.equal(vm.runInContext("'RSA_PKCS1_PADDING' in __nodeBuiltins.crypto.constants",context),false);
 assert.equal(vm.runInContext('(()=>{const fn=()=>1;return __captureAsyncContext(fn)===fn && __runtimeBackendResets.length===0})()',context),true);
});
test('only the selected payload installs its corresponding capabilities',()=>{
 const node=commonContext();vm.runInContext(assembled('runtime_backend.js'),node,{timeout:3000});
 assert.equal(node.__ttRuntimeBackendInstalled,true);
 assert.equal(vm.runInContext('typeof __nodeBuiltins.net.createServer',node),'function');
 assert.equal(vm.runInContext('typeof __nodeBuiltins.crypto.createPrivateKey',node),'function');
 assert.equal(vm.runInContext("typeof __nodeBuiltins['node:vm'].runInNewContext",node),'function');
 assert.equal(vm.runInContext("typeof __nodeBuiltins['node:inspector'].Session",node),'function');
 assert.equal(vm.runInContext("typeof __nodeBuiltins['node:dns/promises'].lookup",node),'function');
 assert.equal(typeof node.DOMPoint,'undefined');
 const dom=commonContext();vm.runInContext(read('runtime_dom.js'),dom,{timeout:3000});
 assert.equal(dom.__ttRuntimeBackendInstalled,false);
 assert.equal(typeof dom.DOMPoint,'function');assert.equal(typeof dom.FileReader,'function');
 assert.throws(()=>vm.runInContext('__nodeBuiltins.child_process',dom),{code:'ERR_TURBO_BACKEND_CAPABILITY'});
});
test('backend task resources use the internal constructor rather than a mutable module export',()=>{
 const context=commonContext();vm.runInContext(assembled('runtime_backend.js'),context,{timeout:3000});
 const answer=vm.runInContext("__nodeBuiltins.async_hooks.AsyncResource=class Replacement{constructor(){throw new Error('public resource replacement invoked');}};__runtimeTaskResource().runInAsyncScope(()=>42,null)",context);
 assert.equal(answer,42);
});
test('TCP internal resources ignore mutable async_hooks exports',()=>{
 const context=commonContext();vm.runInContext(assembled('runtime_backend.js'),context,{timeout:3000});
 const operations=[];
 context.__ttTcp=(operation)=>{operations.push(operation);return JSON.stringify({value:operation==='listen'?{id:1,address:{address:'127.0.0.1',port:12345,family:'IPv4'}}:null});};
 vm.runInContext(`(()=>{const hooks=__nodeBuiltins.async_hooks;const original=hooks.AsyncResource;try{hooks.AsyncResource=class Replacement{constructor(){throw new Error('public resource replacement invoked by TCP');}};const server=__nodeBuiltins.net.createServer();server.listen(0,'127.0.0.1');server.close();}finally{hooks.AsyncResource=original;}})()`,context);
 assert.equal(operations.filter(operation=>operation==='listen').length,1);
 assert.equal(operations.filter(operation=>operation==='close').length,1);
});
