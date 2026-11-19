// node:http ClientRequest body framing as Node v26.11.0 sends it (turbo-test#435): a raw TCP
// server records each request's line, Content-Length / Transfer-Encoding and body bytes.
// GET/HEAD/DELETE/OPTIONS/TRACE carry no framing header by default (useChunkedEncodingByDefault
// is false); other methods send Content-Length for a body given whole to end() and chunked for
// a streamed one; explicit headers win. RECORD=1 node http-request-framing.cjs prints the table.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
const http = require('node:http');

// [name, method, extra headers, steps, body bytes the server waits for when there is no framing]
const shapes = [
  ['GET end()', 'GET', {}, [['end']], 0],
  ["GET end('x')", 'GET', {}, [['end', 'x']], 1],
  ["GET write('ab') end()", 'GET', {}, [['write', 'ab'], ['end']], 2],
  ['HEAD end()', 'HEAD', {}, [['end']], 0],
  ['DELETE end()', 'DELETE', {}, [['end']], 0],
  ["DELETE end('x')", 'DELETE', {}, [['end', 'x']], 1],
  ['OPTIONS end()', 'OPTIONS', {}, [['end']], 0],
  ['TRACE end()', 'TRACE', {}, [['end']], 0],
  ['POST end()', 'POST', {}, [['end']], 0],
  ["POST end('hello')", 'POST', {}, [['end', 'hello']], 0],
  ["POST end(Buffer 'héllo')", 'POST', {}, [['end', Buffer.from('héllo')]], 0],
  ["POST write('a') end('b')", 'POST', {}, [['write', 'a'], ['end', 'b']], 0],
  ["PUT setHeader content-length 3 write('abc') end()", 'PUT', {}, [['setHeader', 'Content-Length', 3], ['write', 'abc'], ['end']], 0],
  ["POST transfer-encoding: chunked end('x')", 'POST', { 'Transfer-Encoding': 'chunked' }, [['end', 'x']], 0],
  ["GET transfer-encoding: chunked write('x') end()", 'GET', { 'Transfer-Encoding': 'chunked' }, [['write', 'x'], ['end']], 0],
  ["PATCH end('é', 'latin1')", 'PATCH', {}, [['end', 'é', 'latin1']], 0],
  ["POST content-length: 2 end('ok')", 'POST', { 'Content-Length': 2 }, [['end', 'ok']], 0],
];

function complete(bytes, hint) {
  const text = bytes.toString('latin1');
  const split = text.indexOf('\r\n\r\n');
  if (split < 0) return false;
  const head = text.slice(0, split);
  const body = text.slice(split + 4);
  if (/\r\ntransfer-encoding: *chunked/i.test(head)) return body.endsWith('0\r\n\r\n');
  const length = /\r\ncontent-length: *(\d+)/i.exec(head);
  return body.length >= (length ? Number(length[1]) : hint);
}
function describe(bytes) {
  const text = bytes.toString('latin1');
  const split = text.indexOf('\r\n\r\n');
  const [line, ...headers] = text.slice(0, split).split('\r\n');
  const value = (name) => headers.filter((h) => h.toLowerCase().startsWith(name + ':')).map((h) => h.slice(name.length + 1).trim());
  return { line, contentLength: value('content-length'), transferEncoding: value('transfer-encoding'), body: text.slice(split + 4) };
}
function recordingServer(hint) {
  let resolveRequest;
  const request = new Promise((resolve) => { resolveRequest = resolve; });
  const server = net.createServer((socket) => {
    let bytes = Buffer.alloc(0);
    socket.on('data', (chunk) => {
      bytes = Buffer.concat([bytes, chunk]);
      if (!complete(bytes, hint)) return;
      resolveRequest(describe(bytes));
      socket.end('HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
    });
  });
  return { server, request };
}
async function send([, method, headers, steps, hint]) {
  const { server, request } = recordingServer(hint);
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const req = http.request({ method, host: '127.0.0.1', port: server.address().port, path: '/p', headers: { Connection: 'close', ...headers } });
  const useChunked = req.useChunkedEncodingByDefault;
  const response = new Promise((resolve, reject) => { req.on('response', (res) => { res.resume(); res.on('end', resolve); }); req.on('error', reject); });
  for (const [step, ...args] of steps) req[step](...args);
  const recorded = await request;
  await response;
  await new Promise((resolve) => server.close(resolve));
  return { useChunkedEncodingByDefault: useChunked, ...recorded };
}

const expected = {
  "GET end()": {
    "useChunkedEncodingByDefault": false,
    "line": "GET /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": ""
  },
  "GET end('x')": {
    "useChunkedEncodingByDefault": false,
    "line": "GET /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": "x"
  },
  "GET write('ab') end()": {
    "useChunkedEncodingByDefault": false,
    "line": "GET /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": "ab"
  },
  "HEAD end()": {
    "useChunkedEncodingByDefault": false,
    "line": "HEAD /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": ""
  },
  "DELETE end()": {
    "useChunkedEncodingByDefault": false,
    "line": "DELETE /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": ""
  },
  "DELETE end('x')": {
    "useChunkedEncodingByDefault": false,
    "line": "DELETE /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": "x"
  },
  "OPTIONS end()": {
    "useChunkedEncodingByDefault": false,
    "line": "OPTIONS /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": ""
  },
  "TRACE end()": {
    "useChunkedEncodingByDefault": false,
    "line": "TRACE /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [],
    "body": ""
  },
  "POST end()": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [
      "0"
    ],
    "transferEncoding": [],
    "body": ""
  },
  "POST end('hello')": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [
      "5"
    ],
    "transferEncoding": [],
    "body": "hello"
  },
  "POST end(Buffer 'héllo')": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [
      "6"
    ],
    "transferEncoding": [],
    "body": "hÃ©llo"
  },
  "POST write('a') end('b')": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [
      "chunked"
    ],
    "body": "1\r\na\r\n1\r\nb\r\n0\r\n\r\n"
  },
  "PUT setHeader content-length 3 write('abc') end()": {
    "useChunkedEncodingByDefault": true,
    "line": "PUT /p HTTP/1.1",
    "contentLength": [
      "3"
    ],
    "transferEncoding": [],
    "body": "abc"
  },
  "POST transfer-encoding: chunked end('x')": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [
      "chunked"
    ],
    "body": "1\r\nx\r\n0\r\n\r\n"
  },
  "GET transfer-encoding: chunked write('x') end()": {
    "useChunkedEncodingByDefault": false,
    "line": "GET /p HTTP/1.1",
    "contentLength": [],
    "transferEncoding": [
      "chunked"
    ],
    "body": "1\r\nx\r\n0\r\n\r\n"
  },
  "PATCH end('é', 'latin1')": {
    "useChunkedEncodingByDefault": true,
    "line": "PATCH /p HTTP/1.1",
    "contentLength": [
      "1"
    ],
    "transferEncoding": [],
    "body": "é"
  },
  "POST content-length: 2 end('ok')": {
    "useChunkedEncodingByDefault": true,
    "line": "POST /p HTTP/1.1",
    "contentLength": [
      "2"
    ],
    "transferEncoding": [],
    "body": "ok"
  }
};

if (process.env.RECORD) {
  (async () => {
    const table = {};
    for (const shape of shapes) table[shape[0]] = await send(shape);
    console.log(JSON.stringify(table, null, 2));
  })();
} else {
  for (const shape of shapes) {
    testCase(shape[0], async () => { assert.deepEqual(await send(shape), expected[shape[0]]); });
  }
}
