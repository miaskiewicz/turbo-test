const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { AsyncLocalStorage } = require('node:async_hooks');
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => { server.off('error', reject); resolve(server.address()); });
  });
}
function close(server) { return new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())); }
function exchange(options, chunks = [], onResponse = () => {}) {
  return new Promise((resolve, reject) => {
    const request = http.request(options, (response) => {
      onResponse(response);
      const bytes = [];
      response.on('data', (chunk) => bytes.push(Buffer.from(chunk)));
      response.on('error', reject);
      response.on('end', () => resolve({ response, bytes: Buffer.concat(bytes) }));
    });
    request.on('error', reject);
    chunks.forEach((chunk) => request.write(chunk));
    request.end();
  });
}
testCase('HTTP listen on zero binds a real ephemeral loopback port and closes', async () => {
  const server = http.createServer((_request, response) => response.end('ok'));
  assert.equal(server.address(), null);
  const address = await listen(server);
  assert.equal(address.address, '127.0.0.1');
  assert.equal(address.family, 'IPv4');
  assert.ok(address.port > 0 && address.port <= 65535);
  await close(server);
  assert.equal(server.address(), null);
});
testCase('HTTP GET delivers method path status headers and real response bytes', async () => {
  let finishResponse; const finished = new Promise((resolve) => { finishResponse = resolve; });
  const server = http.createServer((request, response) => {
    assert.equal(request.method, 'GET'); assert.equal(request.url, '/hello?x=1');
    assert.equal(request.headers['x-input'], 'yes');
    response.statusCode = 201; response.setHeader('X-Reply', 'native');
    response.setHeader('Content-Length', Buffer.from('hello \u{1f680}').length);
    const order = []; response.on('finish', () => order.push('finish'));
    response.on('close', () => { order.push('close'); finishResponse(order); });
    response.end('hello \u{1f680}');
  });
  const address = await listen(server);
  try {
    const result = await exchange({ hostname: '127.0.0.1', port: address.port, path: '/hello?x=1', headers: { 'X-Input': 'yes' } });
    assert.equal(result.response.statusCode, 201);
    assert.equal(result.response.headers['x-reply'], 'native');
    assert.equal(result.bytes.toString(), 'hello \u{1f680}');
    assert.deepEqual(await finished, ['finish', 'close']);
  } finally { await close(server); }
});
testCase('HTTP POST preserves sliced binary writes and request body completion', async () => {
  const server = http.createServer((request, response) => {
    const chunks = [];
    request.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
    request.on('end', () => { assert.equal(request.method, 'POST'); response.end(Buffer.concat(chunks)); });
  });
  const address = await listen(server);
  try {
    const bytes = Buffer.from([9, 0, 255, 128, 9]);
    const result = await exchange({ hostname: '127.0.0.1', port: address.port, method: 'POST', headers: { 'Content-Length': 4 } }, [bytes.subarray(1, 4), Buffer.from([1])]);
    assert.deepEqual(Array.from(result.bytes), [0, 255, 128, 1]);
  } finally { await close(server); }
});
testCase('HTTP chunked transfer delivers first response bytes before final completion', async () => {
  let completed = false;
  const server = http.createServer((_request, response) => {
    response.write('one');
    setTimeout(() => { completed = true; response.end('two'); }, 30);
  });
  const address = await listen(server);
  try {
    const result = await new Promise((resolve, reject) => {
      http.get({ hostname: '127.0.0.1', port: address.port }, (response) => {
        const chunks = []; let first = true;
        response.setEncoding('utf8');
        response.on('data', (chunk) => {
          if (first) { assert.equal(completed, false); first = false; }
          chunks.push(chunk);
        });
        response.on('end', () => resolve(chunks.join('')));
        response.on('error', reject);
      }).on('error', reject);
    });
    assert.equal(result, 'onetwo'); assert.equal(completed, true);
  } finally { await close(server); }
});
testCase('HTTP chunked requests and concurrent connections retain independent bodies', async () => {
  const storage = new AsyncLocalStorage();
  const server = http.createServer((request, response) => {
    assert.equal(storage.getStore(), 'server');
    let body = ''; request.setEncoding('utf8');
    request.on('data', (chunk) => { body += chunk; });
    request.on('end', () => response.end(request.url + ':' + body));
  });
  const address = await storage.run('server', () => listen(server));
  try {
    const results = await Promise.all(Array.from({ length: 4 }, (_, i) => storage.run(i, () => exchange({ hostname: '127.0.0.1', port: address.port, path: '/' + i, method: 'POST', headers: { 'Transfer-Encoding': 'chunked' } }, ['a', String(i)], () => assert.equal(storage.getStore(), i)))));
    assert.deepEqual(results.map((result) => result.bytes.toString()), ['/0:a0', '/1:a1', '/2:a2', '/3:a3']);
  } finally { await close(server); }
});
testCase('HTTP connection failure reports a real error instead of successful empty data', async () => {
  const server = http.createServer(); const address = await listen(server); await close(server);
  await assert.rejects(exchange({ hostname: '127.0.0.1', port: address.port }), { code: 'ECONNREFUSED' });
  assert.throws(() => http.get('https://127.0.0.1/'), { code: 'ERR_INVALID_PROTOCOL' });
});
testCase('HTTP bind conflicts fail asynchronously with EADDRINUSE', async () => {
  const first = http.createServer(); const address = await listen(first);
  const second = http.createServer();
  try {
    const error = await new Promise((resolve) => {
      second.once('error', resolve);
      second.listen(address.port, '127.0.0.1');
    });
    assert.equal(error.code, 'EADDRINUSE'); assert.equal(second.address(), null);
  } finally { await close(first); }
});
testCase('HTTP destroyed requests close without reporting a response or leaking a server', async () => {
  const server = http.createServer((_request, response) => response.end('late'));
  const address = await listen(server); const failure = new Error('cancelled');
  try {
    await new Promise((resolve, reject) => {
      const request = http.request({ hostname: '127.0.0.1', port: address.port });
      request.on('response', () => reject(new Error('unexpected response')));
      request.on('error', (error) => { try { assert.equal(error, failure); } catch (other) { reject(other); } });
      request.on('close', resolve);
      request.end(); request.destroy(failure);
    });
    await delay(1);
  } finally { await close(server); }
  const destroyed = http.createServer((_request, response) => response.destroy());
  const other = await listen(destroyed);
  try { await assert.rejects(exchange({ hostname: '127.0.0.1', port: other.port }), { code: 'ECONNRESET' }); }
  finally { await close(destroyed); }
});
