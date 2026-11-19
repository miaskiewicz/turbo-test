const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const zlib = require('node:zlib');
const { Transform } = require('node:stream');
const { promisify } = require('node:util');
// Bytes compressed by Node's zlib (level 9 raw, default zlib and gzip) for `clear`.
const clear = 'zip-404 stream '.repeat(40);
const node = {
  raw: 'q8os0DUxMFEoLilKTcxVqBrljnKpwQUA',
  zlib: 'eJyryizQNTEwUSguKUpNzFWoGuWOcqnBBQC8U8Oh',
  gzip: 'H4sIAAAAAAAAE6vKLNA1MTBRKC4pSk3MVaga5Y5yqcEFAMNAi21YAgAA',
};
const fromNode = (name) => Buffer.from(node[name], 'base64');
const pairs = [
  ['Deflate', 'Inflate'], ['DeflateRaw', 'InflateRaw'], ['Gzip', 'Gunzip'],
  ['Deflate', 'Unzip'], ['Gzip', 'Unzip'],
];
function collect(stream) {
  const chunks = [];
  stream.on('data', (chunk) => chunks.push(chunk));
  return new Promise((resolve, reject) => {
    stream.on('error', reject);
    stream.on('end', () => resolve(Buffer.concat(chunks)));
  });
}
function writeChunks(stream, text, size) {
  for (let i = 0; i < text.length; i += size) stream.write(text.slice(i, i + size));
  stream.end();
}

test('zlib exposes the stream classes as Transform constructors', () => {
  for (const name of ['Deflate', 'Inflate', 'DeflateRaw', 'InflateRaw', 'Gzip', 'Gunzip', 'Unzip']) {
    assert.equal(typeof zlib[name], 'function', name);
    const stream = new zlib[name]();
    assert.ok(stream instanceof Transform, name);
    assert.ok(zlib['create' + name]() instanceof zlib[name], name);
    stream.close();
  }
});

test('chunked streams round-trip through every compressor and decompressor pair', async () => {
  for (const [encode, decode] of pairs) {
    const encoder = new zlib[encode]({ level: 9 });
    const decoder = new zlib[decode]();
    const compressed = collect(encoder);
    const decoded = collect(encoder.pipe(decoder));
    writeChunks(encoder, clear, 7);
    const bytes = await compressed;
    assert.ok(bytes.length < clear.length, encode);
    assert.equal((await decoded).toString(), clear, encode + '>' + decode);
    assert.equal(encoder.bytesWritten, clear.length, encode);
  }
});

test('decompressors read Node-compressed bytes split at every boundary', async () => {
  for (const [name, decode] of [['raw', 'InflateRaw'], ['zlib', 'Inflate'], ['gzip', 'Gunzip'], ['gzip', 'Unzip'], ['zlib', 'Unzip']]) {
    const bytes = fromNode(name);
    for (const size of [1, 5, bytes.length]) {
      const decoder = new zlib[decode]();
      const out = collect(decoder);
      for (let i = 0; i < bytes.length; i += size) decoder.write(bytes.subarray(i, i + size));
      decoder.end();
      assert.equal((await out).toString(), clear, name + ' by ' + size);
    }
  }
});

test('flush emits a decodable prefix before the stream ends', async () => {
  const deflate = new zlib.DeflateRaw();
  const inflate = new zlib.InflateRaw();
  const seen = [];
  inflate.on('data', (chunk) => seen.push(chunk.toString()));
  deflate.pipe(inflate);
  deflate.write('first half;');
  await new Promise((resolve) => deflate.flush(resolve));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(seen.join(''), 'first half;');
  const done = new Promise((resolve) => inflate.on('end', resolve));
  deflate.end('second half');
  await done;
  assert.equal(seen.join(''), 'first half;second half');
});

test('a DeflateRaw subclass sees every chunk through push and _transform', async () => {
  class Counting extends zlib.DeflateRaw {
    constructor(options) { super(options); this.raw = 0; this.compressed = 0; }
    push(chunk, encoding) { if (chunk) this.compressed += chunk.length; return super.push(chunk, encoding); }
    _transform(chunk, encoding, callback) { this.raw += chunk.length; super._transform(chunk, encoding, callback); }
  }
  const stream = new Counting({ level: 9 });
  const out = collect(stream);
  writeChunks(stream, clear, 64);
  const bytes = await out;
  assert.equal(stream.raw, clear.length);
  assert.equal(stream.compressed, bytes.length);
  assert.equal(zlib.inflateRawSync(bytes).toString(), clear);
});

test('empty input still produces a complete stream', async () => {
  for (const [encode, sync] of [['Deflate', 'inflateSync'], ['DeflateRaw', 'inflateRawSync'], ['Gzip', 'gunzipSync']]) {
    const encoder = new zlib[encode]();
    const out = collect(encoder);
    encoder.end();
    assert.equal(zlib[sync](await out).length, 0, encode);
  }
});

test('malformed and truncated input fails with Node error codes', async () => {
  for (const name of ['Inflate', 'InflateRaw', 'Gunzip', 'Unzip']) {
    const stream = new zlib[name]();
    const out = collect(stream);
    stream.end(Buffer.from([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]));
    await assert.rejects(out, { code: 'Z_DATA_ERROR', errno: -3 }, name);
  }
  for (const [name, bytes] of [['InflateRaw', fromNode('raw')], ['Inflate', fromNode('zlib')], ['Gunzip', fromNode('gzip')], ['Inflate', Buffer.alloc(0)]]) {
    const stream = new zlib[name]();
    const out = collect(stream);
    stream.end(bytes.subarray(0, Math.floor(bytes.length / 2)));
    await assert.rejects(out, { message: 'unexpected end of file', code: 'Z_BUF_ERROR', errno: -5 }, name);
  }
});

test('callback, promisified and sync one-shot helpers use real compression', async () => {
  const input = Buffer.from(clear);
  for (const [encode, decode] of [['deflate', 'inflate'], ['deflateRaw', 'inflateRaw'], ['gzip', 'gunzip'], ['gzip', 'unzip'], ['deflate', 'unzip']]) {
    const packed = await promisify(zlib[encode])(input, { level: 1 });
    assert.ok(packed.length < input.length, encode);
    assert.equal((await promisify(zlib[decode])(packed)).toString(), clear, decode);
    assert.equal(zlib[decode + 'Sync'](zlib[encode + 'Sync'](input)).toString(), clear, decode + 'Sync');
    await new Promise((resolve) => zlib[encode](input, (error, bytes) => {
      assert.equal(error, null);
      assert.equal(zlib[decode + 'Sync'](bytes).toString(), clear);
      resolve();
    }));
  }
  assert.equal(zlib.inflateRawSync(fromNode('raw')).toString(), clear);
  assert.equal(zlib.unzipSync(fromNode('gzip')).toString(), clear);
  await new Promise((resolve) => zlib.inflate(Buffer.from('nope'), (error) => {
    assert.equal(error.code, 'Z_DATA_ERROR');
    resolve();
  }));
});
