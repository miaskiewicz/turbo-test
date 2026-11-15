const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
testCase('empty EOF closes an accepted write-only socket without requiring resume', async () => {
  let acceptedClosed = false; let finishAccepted;
  const acceptedDone = new Promise(resolve => { finishAccepted = resolve; });
  const server = net.createServer(socket => {
    socket.once('close', () => { acceptedClosed = true; finishAccepted(); });
    socket.end('reply');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const client = net.connect({ host: '127.0.0.1', port: server.address().port });
  let response = ''; client.on('data', chunk => { response += chunk; });
  await new Promise(resolve => client.once('close', resolve));
  await new Promise(resolve => server.close(resolve)); await acceptedDone;
  assert.equal(response, 'reply'); assert.equal(acceptedClosed, true);
});
testCase('EOF keeps unconsumed paused bytes until the application resumes', async () => {
  const server = net.createServer(socket => socket.end('buffered'));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const client = net.connect({ host: '127.0.0.1', port: server.address().port });
  await new Promise(resolve => client.once('readable', resolve));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(client.readableEnded, false); assert.equal(client.readableLength, 8);
  let received = ''; const closed = new Promise(resolve => client.once('close', resolve));
  client.on('data', chunk => { received += chunk; }); client.resume();
  await closed; await new Promise(resolve => server.close(resolve));
  assert.equal(received, 'buffered');
});
