const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { EventEmitter } = require('node:events');
function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server.address().port));
  });
}
function close(server) { return new Promise((resolve) => server.close(resolve)); }
function read(response) {
  return new Promise((resolve, reject) => {
    let body = ''; response.setEncoding('utf8');
    response.on('data', (chunk) => body += chunk);
    response.on('end', () => resolve(body)); response.on('error', reject);
  });
}
testCase('HTTP messages share the actual accepted socket and expose connection events', async () => {
  let accepted; let clientSocket; let socketEvents = 0;
  const server = http.createServer((request, response) => {
    assert.equal(request.socket, accepted); assert.equal(response.socket, accepted);
    assert.equal(request.connection, accepted); assert.equal(response.connection, accepted);
    assert.equal(accepted instanceof EventEmitter, true);
    assert.equal(accepted.readable, true); assert.equal(accepted.writable, true);
    assert.equal(accepted.destroyed, false); assert.equal(accepted.remoteAddress, '127.0.0.1');
    response.end('shared-socket');
  });
  server.on('connection', (socket) => { accepted = socket; });
  const port = await listen(server);
  try {
    const body = await new Promise((resolve, reject) => {
      const request = http.get({ hostname: '127.0.0.1', port }, (response) => {
        assert.equal(response.socket, clientSocket); assert.equal(request.socket, clientSocket);
        read(response).then(resolve, reject);
      });
      request.on('socket', (socket) => { clientSocket = socket; socketEvents++; });
      request.on('error', reject);
    });
    assert.equal(body, 'shared-socket'); assert.equal(socketEvents, 1);
  } finally { await close(server); }
});
testCase('HTTP request sockets stay readable while a bounded parser consumes the real body', async () => {
  const server = http.createServer((request, response) => {
    // on-finished uses socket.readable to distinguish an unread body from a closed request.
    assert.equal(request.complete, false); assert.equal(request.socket.readable, true);
    assert.equal(request.readable, true); let bytes = 0;
    request.on('data', (chunk) => { bytes += chunk.length; });
    request.on('end', () => { assert.equal(request.complete, true); response.end(String(bytes)); });
  });
  const port = await listen(server);
  try {
    const result = await new Promise((resolve, reject) => {
      const request = http.request({ hostname: '127.0.0.1', port, method: 'POST' }, (response) => read(response).then(resolve, reject));
      request.on('error', reject); request.end('real-body');
    });
    assert.equal(result, '9');
  } finally { await close(server); }
});
testCase('HTTP socket close marks both directions closed and emits only once', async () => {
  let accepted; let closes = 0; let closed;
  const done = new Promise((resolve) => { closed = resolve; });
  const server = http.createServer((_request, response) => response.end('closed'));
  server.on('connection', (socket) => {
    accepted = socket;
    socket.on('close', (hadError) => { closes++; assert.equal(hadError, false); closed(); });
  });
  const port = await listen(server);
  try {
    await new Promise((resolve, reject) => {
      http.get({ hostname: '127.0.0.1', port }, (response) => read(response).then(resolve, reject)).on('error', reject);
    });
    await done;
    assert.equal(accepted.readable, false); assert.equal(accepted.writable, false);
    assert.equal(accepted.destroyed, true); accepted.destroy(); assert.equal(closes, 1);
  } finally { await close(server); }
});
testCase('Destroying a live HTTP socket closes the native connection and reports the real error', async () => {
  const failure = new Error('socket-cancelled'); let observed; let hadError;
  let closed; const done = new Promise((resolve) => { closed = resolve; });
  const server = http.createServer((_request, response) => response.end('unused'));
  const port = await listen(server);
  try {
    await new Promise((resolve, reject) => {
      const request = http.request({ hostname: '127.0.0.1', port });
      request.on('error', (error) => { try { assert.equal(error, failure); resolve(); } catch (other) { reject(other); } });
      request.on('socket', (socket) => {
        socket.on('error', (error) => { observed = error; });
        socket.on('close', (error) => { hadError = error; closed(); });
        assert.equal(socket.destroy(failure), socket);
        assert.equal(socket.destroyed, true);
      });
    });
    await done; assert.equal(observed, failure); assert.equal(hadError, true);
  } finally { await close(server); }
});
