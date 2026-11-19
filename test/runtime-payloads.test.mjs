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
  // The native drains V8's microtask queue; under node:vm, Node drains it itself after each call.
  __ttRunMicrotasks(){},
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
function terminalTcpContext(terminal) {
 const context=commonContext();vm.runInContext(assembled('runtime_backend.js'),context,{timeout:3000});
 const owned=new Set();const rejected=[];let polls=0;
 const bytes=Buffer.alloc(64*1024,127);bytes[0]=0;bytes[1]=255;
 context.__ttTcp=(operation,encoded)=>{
  const args=JSON.parse(encoded);
  if(operation==='connect'){owned.add(7);return JSON.stringify({value:{id:7}});}
  if(operation==='close'){owned.delete(args.id);return JSON.stringify({value:null});}
  if(operation==='poll'){
   if(polls++===0)return JSON.stringify({value:[{id:7,kind:'connect',address:{localAddress:'127.0.0.1',localPort:12345,localFamily:'IPv4',remoteAddress:'127.0.0.1',remotePort:12346,remoteFamily:'IPv4'}}]});
   owned.delete(7);
   const tail=terminal==='closed'?[{id:7,kind:'end'},{id:7,kind:'closed'}]:[{id:7,kind:'error',error:{code:'ECONNRESET',message:'original native reset'}}];
   return JSON.stringify({value:[{id:7,kind:'finish'},{id:7,kind:'data',data:bytes.toString('base64')},...tail]});
  }
  if(!owned.has(args.id)){rejected.push({operation,id:args.id});return JSON.stringify({error:{code:'ERR_SOCKET_CLOSED',message:'TCP socket is closed'}});}
  return JSON.stringify({value:null});
 };
 return {context,bytes,rejected};
}
test('TCP retires native handles before final data backpressure without reordering stream events',async()=>{
 const {context,bytes,rejected}=terminalTcpContext('closed');
 assert.throws(()=>vm.runInContext("__tcpNative('pause',{id:999,enabled:true})",context),{code:'ERR_SOCKET_CLOSED'});
 const transfer=await vm.runInContext(`(async()=>{
  const socket=new __nodeBuiltins.net.Socket({readableHighWaterMark:8});const chunks=[],events=[];
  socket.on('connect',()=>events.push('connect'));socket.on('data',chunk=>{chunks.push(chunk);events.push('data');});socket.on('end',()=>events.push('end'));
  const done=new Promise((resolve,reject)=>{socket.on('error',reject);socket.once('close',hadError=>{events.push('close');resolve(hadError);});});
  socket.connect({host:'127.0.0.1',port:12346});socket.end();await Promise.resolve();
  __ttTcpPump();__ttTcpPump();const hadError=await done;
  return {bytes:Array.from(Buffer.concat(chunks)),events,hadError,destroyed:socket.destroyed};
 })()`,context);
 assert.deepEqual(Buffer.from(transfer.bytes),bytes);
 assert.deepEqual(Array.from(transfer.events),['connect','data','end','close']);
 assert.equal(transfer.hadError,false);assert.equal(transfer.destroyed,true);
 assert.deepEqual(rejected,[{operation:'pause',id:999}]);
});
test('TCP terminal error batches preserve the native error under buffered backpressure',async()=>{
 const {context,rejected}=terminalTcpContext('error');
 const transfer=await vm.runInContext(`(async()=>{
  const socket=new __nodeBuiltins.net.Socket({readableHighWaterMark:8});const events=[];let failure;
  socket.on('connect',()=>events.push('connect'));socket.on('readable',()=>events.push('readable'));socket.on('error',error=>{failure=error;events.push('error');});
  const done=new Promise(resolve=>socket.once('close',hadError=>{events.push('close');resolve(hadError);}));
  socket.connect({host:'127.0.0.1',port:12346});socket.end();await Promise.resolve();
  __ttTcpPump();__ttTcpPump();const hadError=await done;
  return {events,hadError,code:failure.code,message:failure.message,destroyed:socket.destroyed};
 })()`,context);
 assert.deepEqual(Array.from(transfer.events),['connect','readable','error','close']);
 assert.equal(transfer.code,'ECONNRESET');assert.equal(transfer.message,'original native reset');
 assert.equal(transfer.hadError,true);assert.equal(transfer.destroyed,true);assert.deepEqual(rejected,[]);
});
test('TCP terminal batches keep original errors during public socket reentry',async()=>{
 const {context,rejected}=terminalTcpContext('error');
 const transfer=await vm.runInContext(`(async()=>{
  const socket=new __nodeBuiltins.net.Socket({readableHighWaterMark:8});const events=[];let failure,writeError,writes=0;
  socket.on('readable',()=>{events.push('readable');socket.ref().unref();socket.setNoDelay(true);socket.setKeepAlive(true,1000);socket.pause();socket.resume();socket.write('late',error=>{writeError=error;writes++;});socket.end();});
  socket.on('error',error=>{failure=error;events.push('error');});
  const done=new Promise(resolve=>socket.once('close',hadError=>{events.push('close');resolve(hadError);}));
  socket.connect({host:'127.0.0.1',port:12346});await Promise.resolve();__ttTcpPump();__ttTcpPump();
  const hadError=await done;return {events,hadError,code:failure.code,message:failure.message,writeCode:writeError.code,writes};
 })()`,context);
 assert.deepEqual(Array.from(transfer.events),['readable','error','close']);
 assert.equal(transfer.code,'ECONNRESET');assert.equal(transfer.message,'original native reset');
 assert.equal(transfer.writeCode,'ECONNRESET');assert.equal(transfer.writes,1);assert.equal(transfer.hadError,true);assert.deepEqual(rejected,[]);
});
// Node calls every write callback once: a write the closed handle never sent fails with EPIPE
// (as a real socket's write after the peer closed does) and that error destroys the socket.
test('TCP clean close fails a write its retired handle never sent with EPIPE',async()=>{
 const {context,rejected}=terminalTcpContext('closed');
 const transfer=await vm.runInContext(`(async()=>{
  const socket=new __nodeBuiltins.net.Socket({readableHighWaterMark:8});const events=[];let failure,writeError,writes=0;
  socket.once('readable',()=>{events.push('readable');socket.write('late',error=>{writeError=error;writes++;events.push('write');});socket.end();});
  socket.on('error',error=>{failure=error;events.push('error');});
  const done=new Promise(resolve=>socket.once('close',hadError=>{events.push('close');resolve(hadError);}));
  socket.connect({host:'127.0.0.1',port:12346});await Promise.resolve();__ttTcpPump();__ttTcpPump();
  const hadError=await done;return {events,hadError,code:failure.code,syscall:failure.syscall,writeCode:writeError.code,writes};
 })()`,context);
 assert.deepEqual(Array.from(transfer.events),['readable','write','error','close']);
 assert.equal(transfer.code,'EPIPE');assert.equal(transfer.syscall,'write');
 assert.equal(transfer.writeCode,'EPIPE');assert.equal(transfer.writes,1);assert.equal(transfer.hadError,true);assert.deepEqual(rejected,[]);
});
test('TCP accepts a socket retired in the same native batch without touching its dead handle',async()=>{
 const context=commonContext();vm.runInContext(assembled('runtime_backend.js'),context,{timeout:3000});
 const rejected=[];const owned=new Set([1]);
 context.__ttTcp=(operation,encoded)=>{
  const args=JSON.parse(encoded);
  if(operation==='listen')return JSON.stringify({value:{id:1,address:{address:'127.0.0.1',port:12345,family:'IPv4'}}});
  if(operation==='close'){owned.delete(args.id);return JSON.stringify({value:null});}
  if(operation==='poll')return JSON.stringify({value:[{kind:'accept',server:1,id:7,address:{localAddress:'127.0.0.1',localPort:12345,localFamily:'IPv4',remoteAddress:'127.0.0.1',remotePort:12346,remoteFamily:'IPv4'}},{kind:'data',id:7,data:Buffer.alloc(65536,127).toString('base64')},{kind:'error',id:7,error:{code:'ECONNRESET',message:'accepted native reset'}}]});
  if(!owned.has(args.id)){rejected.push({operation,id:args.id});return JSON.stringify({error:{code:'ERR_SOCKET_CLOSED',message:'TCP socket is closed'}});}
  return JSON.stringify({value:null});
 };
 const transfer=await vm.runInContext(`(async()=>{
  const events=[];let failure;let finish;
  const done=new Promise(resolve=>{finish=resolve;});
  const server=__nodeBuiltins.net.createServer({noDelay:true,keepAlive:true,readableHighWaterMark:8},socket=>{
   events.push('connection');socket.on('readable',()=>{events.push('readable');socket.ref().unref();socket.setNoDelay(true);socket.setKeepAlive(false);});
   socket.on('error',error=>{failure=error;events.push('error');});socket.once('close',hadError=>{events.push('close');finish(hadError);});
  });
  server.listen(0,'127.0.0.1');__ttTcpPump();const hadError=await done;
  await new Promise(resolve=>server.close(resolve));
  return {events,hadError,code:failure.code,message:failure.message};
 })()`,context);
 assert.deepEqual(Array.from(transfer.events),['connection','readable','error','close']);
 assert.equal(transfer.hadError,true);assert.equal(transfer.code,'ECONNRESET');assert.equal(transfer.message,'accepted native reset');assert.deepEqual(rejected,[]);
});
