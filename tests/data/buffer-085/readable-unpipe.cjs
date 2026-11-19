const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const { PassThrough, Readable, Writable } = require('node:stream');
// multer's done() calls req.unpipe(busboy) before calling next (#418).
const tick = () => new Promise((resolve) => setTimeout(resolve, 10));
function sink(name, log) {
  const stream = new Writable({ write(chunk, _encoding, callback) { log.push(name + ':' + chunk); callback(); } });
  stream.on('unpipe', (source) => log.push(name + ':unpipe:' + (source === stream.source)));
  stream.on('finish', () => log.push(name + ':finish'));
  return stream;
}
test('unpipe(dest) detaches one destination and emits unpipe on it', async () => {
  const log = [], src = new PassThrough(), a = sink('a', log), b = sink('b', log);
  a.source = src; b.source = src;
  src.pipe(a); src.pipe(b);
  src.write('1');
  await tick();
  assert.equal(src.unpipe(a), src);
  src.write('2');
  src.end();
  await tick();
  assert.deepEqual(log, ['a:1', 'b:1', 'a:unpipe:true', 'b:2', 'b:finish', 'b:unpipe:true']);
});
test('unpipe() detaches every destination and pauses the source', async () => {
  const log = [], src = new PassThrough(), a = sink('a', log), b = sink('b', log);
  a.source = src; b.source = src;
  src.pipe(a); src.pipe(b);
  assert.equal(src.unpipe(), src);
  assert.equal(src.isPaused(), true);
  src.end('late');
  await tick();
  assert.deepEqual(log, ['a:unpipe:true', 'b:unpipe:true']);
  assert.equal(src.read().toString(), 'late');
});
test('unpipe of a stream that is not piped is a no-op', () => {
  const src = new Readable({ read() {} }), other = new PassThrough();
  let unpiped = 0;
  other.on('unpipe', () => unpiped++);
  assert.equal(src.unpipe(other), src);
  assert.equal(src.unpipe(), src);
  assert.equal(unpiped, 0);
  assert.equal(typeof PassThrough.prototype.unpipe, 'function');
});
