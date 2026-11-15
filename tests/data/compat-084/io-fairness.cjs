const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const net = require('node:net');
const http = require('node:http');
testCase('continuous child output does not starve TCP HTTP or deadline timers', async () => {
  const tcp = net.createServer(socket => socket.end('tcp-ready'));
  const web = http.createServer((request, response) => response.end('http-ready'));
  await new Promise(resolve => tcp.listen(0, '127.0.0.1', resolve));
  await new Promise(resolve => web.listen(0, '127.0.0.1', resolve));
  const source = "const end=Date.now()+2000; const chunk=Buffer.alloc(65536,120); function pump(){if(Date.now()>=end)return; if(process.stdout.write(chunk))setImmediate(pump); else process.stdout.once('drain',pump);} pump();";
  const child = spawn(process.execPath, ['-e', source], { env: {} });
  child.stdout.resume(); child.stderr.resume();
  const closed = new Promise(resolve => child.once('close', resolve));
  try {
    await new Promise(resolve => child.stdout.once('data', resolve));
    const timer = new Promise(resolve => setTimeout(() => resolve('timer-ready'), 10));
    const raw = new Promise((resolve, reject) => {
      const socket = net.connect({host:'127.0.0.1',port:tcp.address().port}); let text = '';
      socket.on('data', chunk => { text += chunk; }); socket.on('end', () => resolve(text)); socket.on('error', reject);
    });
    const response = new Promise((resolve, reject) => {
      http.get({hostname:'127.0.0.1',port:web.address().port,path:'/'}, incoming => {
        let text = ''; incoming.on('data', chunk => { text += chunk; }); incoming.on('end', () => resolve(text));
      }).on('error', reject);
    });
    assert.deepEqual(await Promise.all([timer, raw, response]), ['timer-ready','tcp-ready','http-ready']);
    assert.equal(child.exitCode, null, 'all backends must progress before the output producer exits');
  } finally {
    child.kill(); await closed;
    await new Promise(resolve => tcp.close(resolve)); await new Promise(resolve => web.close(resolve));
  }
});
