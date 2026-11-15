const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const hooks = require('node:async_hooks');
const net = require('node:net');
testCase('TCP delivery uses internal resources when async_hooks exports are reassigned', async () => {
  const Original = hooks.AsyncResource;
  let server; let client; let peer;
  hooks.AsyncResource = class ExportedResourceReplacement {
    constructor() { throw new Error('public resource replacement must not be used by TCP'); }
  };
  try {
    server = net.createServer(socket => { peer = socket; socket.end('internal TCP bytes'); });
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
    client = new net.Socket(); const chunks = [];
    const completed = new Promise((resolve, reject) => { client.on('error', reject); client.on('data', bytes => chunks.push(bytes)); client.once('close', resolve); });
    client.connect({ host: '127.0.0.1', port: server.address().port });
    await completed; assert.equal(Buffer.concat(chunks).toString(), 'internal TCP bytes');
  } finally {
    hooks.AsyncResource = Original;
    if (client) client.destroy();
    if (peer) peer.destroy();
    if (server && server.listening) await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});
