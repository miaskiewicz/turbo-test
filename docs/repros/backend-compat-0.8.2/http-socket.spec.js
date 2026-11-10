test('binds a real loopback HTTP port', async () => {
  const http = require('node:http');
  const server = http.createServer((_req, res) => res.end('ok'));
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    expect(server.address()?.port).toBeGreaterThan(0);
  } finally {
    server.close();
  }
}, 1000);
