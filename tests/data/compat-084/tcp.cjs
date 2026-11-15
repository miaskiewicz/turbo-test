const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
const stream = require('node:stream');
function listening(server, host) { return new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, host, resolve); }); }
function closedServer(server) { return new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
function connected(socket, options) { return new Promise((resolve, reject) => { socket.once('error', reject); socket.connect(options, resolve); }); }
function closedSocket(socket) { return new Promise(resolve => socket.once('close', resolve)); }
async function bytesExchange(host, family) {
  const payload = Buffer.from([0, 255, 128, 1, 13, 10, 0, 42]);
  const events = []; let accepted;
  const server = net.createServer(socket => {
    accepted = socket;
    assert.ok(socket instanceof net.Socket); assert.ok(socket instanceof stream.Duplex);
    socket.on('error', error => { throw error; });
    socket.on('data', chunk => socket.write(chunk));
  });
  await listening(server, host);
  const address = server.address();
  assert.ok(address.port > 0); assert.equal(address.address, host); assert.equal(address.family, family);
  const client = new net.Socket(); const chunks = [];
  client.on('data', chunk => { events.push('data'); chunks.push(chunk); });
  client.on('end', () => events.push('end')); const done = closedSocket(client);
  client.on('close', () => events.push('close'));
  try {
    await connected(client, { host, port: address.port }); events.push('connect');
    assert.equal(client.remotePort, address.port); assert.ok(client.localPort > 0);
    assert.equal(client.remoteAddress, host); assert.equal(client.remoteFamily, family);
    assert.equal(client.setNoDelay(false), client); assert.equal(client.setNoDelay(), client);
    client.end(payload); await done;
    assert.deepEqual(Buffer.concat(chunks), payload);
    assert.ok(events.indexOf('connect') < events.indexOf('data'));
    assert.ok(events.indexOf('data') < events.indexOf('end'));
    assert.ok(events.indexOf('end') < events.indexOf('close'));
    assert.equal(client.destroyed, true); assert.equal(client.connecting, false);
  } finally { client.destroy(); if (accepted) accepted.destroy(); await closedServer(server); }
}
testCase('real IPv4 TCP sockets exchange exact bytes and ordered lifecycle events', () => bytesExchange('127.0.0.1', 'IPv4'));
testCase('real IPv6 TCP sockets exchange exact bytes and expose actual addresses', () => bytesExchange('::1', 'IPv6'));
testCase('TCP read EOF permits an explicit half-open response', async () => {
  let accepted; let received = ''; let writableAtEnd;
  const server = net.createServer({ allowHalfOpen: true }, socket => {
    accepted = socket; socket.setEncoding('utf8'); socket.on('data', chunk => { received += chunk; });
    socket.on('end', () => { writableAtEnd = socket.writable; socket.end('tail'); });
  });
  await listening(server, '127.0.0.1');
  const client = net.createConnection({ host: '127.0.0.1', port: server.address().port });
  let reply = ''; client.setEncoding('utf8'); client.on('data', chunk => { reply += chunk; });
  const done = closedSocket(client); client.on('error', error => { throw error; });
  try { client.end('head'); await done; assert.equal(received, 'head'); assert.equal(reply, 'tail'); assert.equal(writableAtEnd, true); }
  finally { client.destroy(); if (accepted) accepted.destroy(); await closedServer(server); }
});
testCase('TCP server close waits for its real connection and reports connection counts', async () => {
  let accept; const accepted = new Promise(resolve => { accept = resolve; });
  const server = net.createServer(socket => accept(socket)); await listening(server, '127.0.0.1');
  const client = new net.Socket(); await connected(client, { port: server.address().port, host: '127.0.0.1' });
  const peer = await accepted;
  try {
    assert.equal(await new Promise((resolve, reject) => server.getConnections((error, count) => error ? reject(error) : resolve(count))), 1);
    let serverClosed = false; const done = new Promise(resolve => server.close(() => { serverClosed = true; resolve(); }));
    assert.equal(server.listening, false); assert.equal(server.address(), null); assert.equal(serverClosed, false);
    const clientDone = closedSocket(client); client.end(); peer.resume(); client.resume(); await clientDone; await done;
    assert.equal(serverClosed, true);
  } finally { client.destroy(); peer.destroy(); }
});
testCase('TCP refused connections emit a real error before close instead of pretending to connect', async () => {
  const reservation = net.createServer(); await listening(reservation, '127.0.0.1');
  const port = reservation.address().port; await closedServer(reservation);
  const client = new net.Socket(); const events = []; let failure; let hadError;
  client.on('error', error => { events.push('error'); failure = error; });
  const done = new Promise(resolve => client.on('close', error => { events.push('close'); hadError = error; resolve(); }));
  client.connect({ host: '127.0.0.1', port }); await done;
  assert.equal(failure.code, 'ECONNREFUSED'); assert.deepEqual(events, ['error', 'close']);
  assert.equal(hadError, true); assert.equal(client.destroyed, true);
});
testCase('TCP explicit destroy preserves the supplied error and closes only once', async () => {
  let accept; const accepted = new Promise(resolve => { accept = resolve; });
  const server = net.createServer(socket => accept(socket)); await listening(server, '127.0.0.1');
  const client = new net.Socket(); await connected(client, { port: server.address().port, host: '127.0.0.1' }); const peer = await accepted;
  const failure = new Error('owned TCP shutdown'); let received; let closes = 0;
  client.on('error', error => { received = error; }); client.on('close', () => { closes++; }); const done = closedSocket(client);
  try { assert.equal(client.destroy(failure), client); client.destroy(); await done; assert.equal(received, failure); assert.equal(closes, 1); }
  finally { peer.destroy(); await closedServer(server); }
});
testCase('TCP writable backpressure drains while preserving a bounded large payload', async () => {
  const payload = Buffer.alloc(1024 * 1024, 127); payload[0] = 0; payload[1] = 255;
  let accepted;
  const server = net.createServer({ allowHalfOpen: true }, socket => {
    accepted = socket; const chunks = []; socket.on('data', chunk => chunks.push(chunk));
    socket.on('end', () => socket.end(Buffer.concat(chunks)));
  }); await listening(server, '127.0.0.1');
  const client = new net.Socket({ writableHighWaterMark: 1024 }); const reply = []; let writes = 0; let drains = 0;
  client.on('data', chunk => reply.push(chunk)); client.on('error', error => { throw error; }); const done = closedSocket(client);
  try {
    await connected(client, { host: '127.0.0.1', port: server.address().port });
    client.once('drain', () => { drains++; client.end(); });
    assert.equal(client.write(payload, () => { writes++; }), false);
    assert.equal(client.writableNeedDrain, true); await done;
    assert.equal(writes, 1); assert.equal(drains, 1); assert.deepEqual(Buffer.concat(reply), payload);
  } finally { client.destroy(); if (accepted) accepted.destroy(); await closedServer(server); }
});
testCase('TCP and TLS retain distinct constructors and stable module identities', () => {
  const tls = require('node:tls'); assert.equal(net, require('net')); assert.equal(net.default || net, net);
  assert.equal(net.createConnection, net.connect); assert.notEqual(tls.Server, net.Server);
  assert.equal(net.Server.prototype.isPrototypeOf(tls.Server.prototype), true);
});
testCase('TCP readiness progresses while user immediates keep rescheduling', async () => {
  const payload = Buffer.from('ready alongside check phase'); let peer; let spinning = true; let immediate;
  const spin = () => { if (spinning) immediate = setImmediate(spin); };
  const server = net.createServer(socket => { peer = socket; socket.end(payload); });
  await listening(server, '127.0.0.1'); const client = new net.Socket(); const chunks = [];
  client.on('error', error => { throw error; }); client.on('data', bytes => chunks.push(bytes)); const done = closedSocket(client);
  try { spin(); client.connect({ host: '127.0.0.1', port: server.address().port }); await done; assert.deepEqual(Buffer.concat(chunks), payload); }
  finally { spinning = false; clearImmediate(immediate); client.destroy(); if (peer) peer.destroy(); await closedServer(server); }
});
testCase('TCP keepalive and no-delay options apply before and after connecting', async () => {
  let peer; const server = net.createServer(socket => { peer = socket; socket.end('options'); });
  await listening(server, '127.0.0.1'); const client = new net.Socket(); const chunks = [];
  client.on('data', bytes => chunks.push(bytes)); client.on('error', error => { throw error; }); const done = closedSocket(client);
  assert.equal(client.setKeepAlive(true, 1000), client); assert.equal(client.setNoDelay(false), client);
  try {
    await connected(client, { host: '127.0.0.1', port: server.address().port });
    assert.equal(client.setKeepAlive(false), client); assert.equal(client.setKeepAlive(true, 2000), client);
    await done; assert.equal(Buffer.concat(chunks).toString(), 'options');
  } finally { client.destroy(); if (peer) peer.destroy(); await closedServer(server); }
});
