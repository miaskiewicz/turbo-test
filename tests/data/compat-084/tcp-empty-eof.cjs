const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
testCase('empty EOF closes an accepted write-only socket without requiring resume', async () => {
  let acceptedClosed = false;
  const server = net.createServer(socket => {
    socket.once('close', () => { acceptedClosed = true; });
    socket.end('reply');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const client = net.connect({ host: '127.0.0.1', port: server.address().port });
  let response = ''; client.on('data', chunk => { response += chunk; });
  await new Promise(resolve => client.once('close', resolve));
  await new Promise(resolve => server.close(resolve));
  assert.equal(response, 'reply'); assert.equal(acceptedClosed, true);
});
