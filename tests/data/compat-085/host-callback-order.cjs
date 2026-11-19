// After every callback the host makes into JS (a socket, child pipe, HTTP or MessagePort event,
// a timer, an immediate), Node v26.11.0 runs the nextTicks it queued, then its promise jobs,
// before the next callback. Each event logs itself plus a nextTick and a microtask marker.
// RECORD=1 node host-callback-order.cjs prints the table this file asserts.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
const http = require('node:http');
const { spawn } = require('node:child_process');
const { MessageChannel } = require('node:worker_threads');

const mark = (log, label) => {
  log.push(label);
  process.nextTick(() => log.push(label + ':tick'));
  Promise.resolve().then(() => log.push(label + ':micro'));
};
const on = (log, emitter, event, label = event) => emitter.on(event, () => mark(log, label));
const wait = (emitter, event) => new Promise((resolve) => emitter.once(event, resolve));

const scenarios = {
  async 'TCP socket events drain between callbacks'(log) {
    const server = net.createServer((socket) => socket.end('ab'));
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const client = net.connect(server.address().port, '127.0.0.1');
    client.setEncoding('utf8');
    on(log, client, 'connect');
    client.on('data', (chunk) => mark(log, 'data ' + chunk));
    on(log, client, 'end');
    on(log, client, 'close');
    await wait(client, 'close');
    await new Promise((resolve) => server.close(resolve));
  },
  async 'child process pipe and exit events drain between callbacks'(log) {
    const child = spawn('/bin/sh', ['-c', 'printf ab; exec 1>&-; sleep 0.2']);
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (chunk) => mark(log, 'stdout data ' + chunk));
    on(log, child.stdout, 'end', 'stdout end');
    child.on('exit', (code) => mark(log, 'exit ' + code));
    await wait(child, 'close');
  },
  async 'HTTP request and response events drain between callbacks'(log) {
    const server = http.createServer((req, res) => { res.setHeader('connection', 'close'); res.end('ok'); });
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const req = http.get({ port: server.address().port, host: '127.0.0.1', headers: { connection: 'close' } });
    const res = await new Promise((resolve) => req.on('response', (response) => { mark(log, 'response'); log.push('framing ' + response.headers['content-length'] + ' ' + response.headers['transfer-encoding']); resolve(response); }));
    res.setEncoding('utf8');
    res.on('data', (chunk) => mark(log, 'data ' + chunk));
    on(log, res, 'end');
    await wait(res, 'end');
    await new Promise((resolve) => setTimeout(resolve, 20));
    await new Promise((resolve) => server.close(resolve));
  },
  async 'MessagePort messages drain between deliveries'(log) {
    const { port1, port2 } = new MessageChannel();
    let seen = 0;
    const done = new Promise((resolve) => port2.on('message', (value) => { mark(log, 'message ' + value); if (++seen === 2) resolve(); }));
    port1.postMessage('a'); port1.postMessage('b');
    await done;
    port1.close();
    await new Promise((resolve) => setTimeout(resolve, 10));
  },
  async 'timers due together drain between callbacks'(log) {
    await new Promise((resolve) => {
      setTimeout(() => mark(log, 'timer a'), 5);
      setTimeout(() => { mark(log, 'timer b'); setTimeout(resolve, 5); }, 5);
    });
  },
  async 'immediates drain between callbacks'(log) {
    await new Promise((resolve) => {
      setImmediate(() => mark(log, 'immediate a'));
      setImmediate(() => { mark(log, 'immediate b'); setTimeout(resolve, 5); });
    });
  },
};

const expected = {
  "TCP socket events drain between callbacks": [
    "connect",
    "connect:tick",
    "connect:micro",
    "data ab",
    "data ab:tick",
    "data ab:micro",
    "end",
    "end:tick",
    "end:micro",
    "close",
    "close:tick",
    "close:micro"
  ],
  "child process pipe and exit events drain between callbacks": [
    "stdout data ab",
    "stdout data ab:tick",
    "stdout data ab:micro",
    "stdout end",
    "stdout end:tick",
    "stdout end:micro",
    "exit 0",
    "exit 0:tick",
    "exit 0:micro"
  ],
  "HTTP request and response events drain between callbacks": [
    "response",
    "framing 2 undefined",
    "response:tick",
    "response:micro",
    "data ok",
    "end",
    "data ok:tick",
    "end:tick",
    "data ok:micro",
    "end:micro"
  ],
  "MessagePort messages drain between deliveries": [
    "message a",
    "message a:tick",
    "message a:micro",
    "message b",
    "message b:tick",
    "message b:micro"
  ],
  "timers due together drain between callbacks": [
    "timer a",
    "timer a:tick",
    "timer a:micro",
    "timer b",
    "timer b:tick",
    "timer b:micro"
  ],
  "immediates drain between callbacks": [
    "immediate a",
    "immediate a:tick",
    "immediate a:micro",
    "immediate b",
    "immediate b:tick",
    "immediate b:micro"
  ]
};

if (process.env.RECORD) {
  (async () => {
    const table = {};
    for (const [name, run] of Object.entries(scenarios)) { const log = []; await run(log); table[name] = log; }
    console.log(JSON.stringify(table, null, 2));
  })();
} else {
  for (const [name, run] of Object.entries(scenarios)) {
    testCase(name, async () => {
      const log = [];
      await run(log);
      assert.deepEqual(log, expected[name]);
    });
  }
}
