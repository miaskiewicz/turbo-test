const assert = require('node:assert/strict');
const http = require('node:http');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

async function exchange(handler, configure) {
  const server = http.createServer(handler);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  let request;
  try {
    return await new Promise((resolve, reject) => {
      request = http.request({ hostname: '127.0.0.1', port: server.address().port }, response => {
        response.resume(); response.once('error', reject);
        response.once('end', resolve);
      });
      request.once('error', reject); configure(request); request.end();
    });
  } finally {
    if (request) request.destroy();
    await new Promise(resolve => server.close(resolve));
  }
}

test('HTTP request timeout zero disables an existing deadline', async () => {
  let timeouts = 0;
  await exchange((_request, response) => setTimeout(() => response.end('done'), 80), request => {
    request.setTimeout(20, () => timeouts++);
    assert.equal(request.setTimeout(0), request);
  });
  assert.equal(timeouts, 0);
});

test('HTTP request timeout replacement cancels the previous deadline', async () => {
  let timeouts = 0;
  await exchange((_request, response) => setTimeout(() => response.end('done'), 80), request => {
    request.on('timeout', () => timeouts++);
    request.setTimeout(20); request.setTimeout(500);
  });
  assert.equal(timeouts, 0);
});

test('HTTP request timeout fires once for an idle connection', async () => {
  let timeouts = 0;
  await exchange((_request, response) => setTimeout(() => response.end('done'), 100), request => {
    request.setTimeout(25, () => timeouts++);
  });
  assert.equal(timeouts, 1);
});

test('HTTP request timeout refreshes when response bytes arrive', async () => {
  let timeouts = 0;
  await exchange(async (_request, response) => {
    response.write('first');
    for (let index = 0; index < 4; index++) {
      await wait(30); response.write('next');
    }
    response.end();
  }, request => request.setTimeout(90, () => timeouts++));
  assert.equal(timeouts, 0);
});

test('HTTP request completion and destruction cancel remaining deadlines', async () => {
  let timeouts = 0;
  let saved;
  await exchange((_request, response) => response.end('done'), request => {
    saved = request; request.setTimeout(30, () => timeouts++);
  });
  saved.setTimeout(30);
  await wait(70);
  assert.equal(timeouts, 0);
});
