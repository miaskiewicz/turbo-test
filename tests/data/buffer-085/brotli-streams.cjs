const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const zlib = require('node:zlib');
const { Transform } = require('node:stream');
const { promisify } = require('node:util');
const C = zlib.constants;
const clear = 'brotli-085 stream '.repeat(40);
// Node 26.11's brotliCompressSync(clear).
const fromNode = Buffer.from('G88C+B2pU587XsPU4IjJFjm6TGN7K4IKqxD2wX2QC+IB', 'base64');
const OUTPUT = __OUTPUT__;
function collect(stream) {
  const chunks = [];
  stream.on('data', (chunk) => chunks.push(chunk));
  return new Promise((resolve, reject) => {
    stream.on('error', reject);
    stream.on('end', () => resolve(Buffer.concat(chunks)));
  });
}

test('Brotli stream classes are Transform constructors', () => {
  for (const name of ['BrotliCompress', 'BrotliDecompress']) {
    assert.equal(typeof zlib[name], 'function');
    const stream = new zlib[name]();
    assert.ok(stream instanceof Transform);
    assert.ok(zlib['create' + name]() instanceof zlib[name]);
    stream.close();
  }
});
test('chunked compression round-trips and Node-made bytes decode at every split', async () => {
  const encoder = zlib.createBrotliCompress();
  const packed = collect(encoder);
  for (let i = 0; i < clear.length; i += 7) encoder.write(clear.slice(i, i + 7));
  encoder.end();
  const bytes = await packed;
  assert.ok(bytes.length < clear.length);
  for (const input of [bytes, fromNode]) {
    for (const size of [1, 5, input.length]) {
      const decoder = zlib.createBrotliDecompress();
      const out = collect(decoder);
      for (let i = 0; i < input.length; i += size) decoder.write(input.subarray(i, i + size));
      decoder.end();
      assert.equal((await out).toString(), clear, 'split ' + size);
    }
  }
});
test('params set quality, window, mode and size hint', () => {
  const outputs = {};
  for (const [name, params] of Object.entries({
    q0: { [C.BROTLI_PARAM_QUALITY]: 0 },
    q11: { [C.BROTLI_PARAM_QUALITY]: 11 },
    lgwin10: { [C.BROTLI_PARAM_LGWIN]: 10 },
    text: { [C.BROTLI_PARAM_MODE]: C.BROTLI_MODE_TEXT, [C.BROTLI_PARAM_SIZE_HINT]: clear.length },
    clamped: { [C.BROTLI_PARAM_QUALITY]: 12, [C.BROTLI_PARAM_LGWIN]: 30, [C.BROTLI_PARAM_MODE]: 9 },
    boolean: { [C.BROTLI_PARAM_QUALITY]: true },
  })) {
    const bytes = zlib.brotliCompressSync(clear, { params });
    assert.equal(zlib.brotliDecompressSync(bytes).toString(), clear, name);
    outputs[name] = bytes.toString('base64');
  }
  assert.ok(Buffer.from(outputs.q11, 'base64').length <= Buffer.from(outputs.q0, 'base64').length);
  outputs.empty = zlib.brotliCompressSync('').toString('base64');
  fs.writeFileSync(OUTPUT, JSON.stringify(outputs));
});
test('flush emits a decodable prefix before the stream ends', async () => {
  const encoder = zlib.createBrotliCompress();
  const decoder = zlib.createBrotliDecompress();
  const seen = [];
  decoder.on('data', (chunk) => seen.push(chunk.toString()));
  encoder.pipe(decoder);
  encoder.write('first half;');
  await new Promise((resolve) => encoder.flush(resolve));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(seen.join(''), 'first half;');
  const done = new Promise((resolve) => decoder.on('end', resolve));
  encoder.end('second half');
  await done;
  assert.equal(seen.join(''), 'first half;second half');
});
test('empty input, trailing bytes and helpers', async () => {
  assert.equal(zlib.brotliDecompressSync(zlib.brotliCompressSync(Buffer.alloc(0))).length, 0);
  assert.equal(zlib.brotliDecompressSync(Buffer.concat([fromNode, Buffer.from('xyz')])).toString(), clear);
  const packed = await promisify(zlib.brotliCompress)(Buffer.from(clear), { params: { [C.BROTLI_PARAM_QUALITY]: 5 } });
  assert.equal((await promisify(zlib.brotliDecompress)(packed)).toString(), clear);
  await new Promise((resolve) => zlib.brotliCompress(clear, (error, bytes) => {
    assert.equal(error, null);
    assert.equal(zlib.brotliDecompressSync(bytes).toString(), clear);
    resolve();
  }));
});
test('malformed and truncated input fail with Node codes', async () => {
  const fails = (input, props) => assert.throws(() => zlib.brotliDecompressSync(input), (e) => Object.entries(props).every(([k, v]) => e[k] === v));
  fails(Buffer.from('nope nope nope'), { code: 'ERR__ERROR_FORMAT_PADDING_2', errno: -15, message: 'Decompression failed' });
  fails(fromNode.subarray(0, fromNode.length - 3), { code: 'Z_BUF_ERROR', errno: -5, message: 'unexpected end of file' });
  fails(Buffer.alloc(0), { code: 'Z_BUF_ERROR', errno: -5, message: 'unexpected end of file' });
  const stream = zlib.createBrotliDecompress();
  const out = collect(stream);
  stream.end(Buffer.from('nope nope'));
  await assert.rejects(out, { code: 'ERR__ERROR_FORMAT_PADDING_2', errno: -15, message: 'Decompression failed' });
});
test('params are validated like Node', () => {
  assert.throws(() => zlib.createBrotliCompress({ params: { 9: 0 } }), { name: 'RangeError', code: 'ERR_BROTLI_INVALID_PARAM', message: '9 is not a valid Brotli parameter' });
  assert.throws(() => zlib.createBrotliCompress({ params: { x: 0 } }), { name: 'RangeError', code: 'ERR_BROTLI_INVALID_PARAM', message: 'x is not a valid Brotli parameter' });
  assert.throws(() => zlib.createBrotliCompress({ params: { '1.5': 0 } }), { code: 'ERR_BROTLI_INVALID_PARAM', message: '1.5 is not a valid Brotli parameter' });
  assert.throws(() => zlib.createBrotliCompress({ params: { 1: 'x' } }), { name: 'TypeError', code: 'ERR_INVALID_ARG_TYPE', message: 'The "options.params[key]" property must be of type number. Received type string (\'x\')' });
  assert.throws(() => zlib.createBrotliCompress({ params: { [C.BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING]: 2 } }), { code: 'ERR_ZLIB_INITIALIZATION_FAILED', message: 'Initialization failed' });
  assert.throws(() => zlib.createBrotliDecompress({ params: { [C.BROTLI_PARAM_SIZE_HINT]: 1 } }), { code: 'ERR_ZLIB_INITIALIZATION_FAILED', message: 'Initialization failed' });
  assert.equal(zlib.brotliDecompressSync(fromNode, { params: { [C.BROTLI_DECODER_PARAM_LARGE_WINDOW]: 1 } }).toString(), clear);
});
