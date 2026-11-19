//! Buffer#compare (turbo-test#408, csv-parse BOM detection) and Buffer#write (turbo-test#409,
//! pg-protocol buffer-writer). Every expectation was taken from Node 26.11, and this same spec
//! passes under real vitest on Node.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r##"
const caught = (operation) => { try { return operation(); } catch (error) { return [error.name, error.code, error.message]; } };
const written = (size, fill, ...args) => caught(() => { const b = Buffer.alloc(size, fill); return [b.write(...args), Array.from(b)]; });
const range = (field, rule, received) => ['RangeError', 'ERR_OUT_OF_RANGE', `The value of "${field}" is out of range. It must be ${rule}. Received ${received}`];
const type = (field, received) => ['TypeError', 'ERR_INVALID_ARG_TYPE', `The "${field}" argument must be of type number. Received ${received}`];
const target = (received) => ['TypeError', 'ERR_INVALID_ARG_TYPE', `The "target" argument must be an instance of Buffer or Uint8Array. Received ${received}`];
test('instance compare detects a CSV byte-order mark the way csv-parse calls it', () => {
 const bom = Buffer.from([0xef, 0xbb, 0xbf]);
 expect(bom.compare(Buffer.from([0xef, 0xbb, 0xbf, 0x61]), 0, 3)).toBe(0);
 expect(bom.compare(Buffer.from('abc'), 0, 3)).toBe(1);
 expect(caught(() => bom.compare(Buffer.from([0xef, 0xbb]), 0, 3))).toEqual(range('targetEnd', '>= 0 && <= 2', 3));
});
test('instance compare is unsigned lexicographic over whole buffers and views', () => {
 const pairs = [[[1, 2], [1, 2], 0], [[1, 2], [1, 3], -1], [[1, 3], [1, 2], 1], [[1], [1, 0], -1], [[1, 0], [1], 1], [[], [], 0], [[255], [1], 1]];
 for (const [a, b, want] of pairs) expect(Buffer.from(a).compare(new Uint8Array(b))).toBe(want);
 expect(Buffer.from([1, 2]).compare(new Uint8Array([9, 1, 2, 9]).subarray(1, 3))).toBe(0);
 expect(Buffer.from([9, 1, 2, 9]).subarray(1, 3).compare(Buffer.from([1, 2]))).toBe(0);
});
test('instance compare windows and empty-range ordering match Node', () => {
 const abc = [1, 2, 3];
 const cases = [[[1, 2, 3, 4], [0, 2, 3, 9], [1, 3, 1, 3], 0], [[1, 2, 3, 4], [2, 3], [0, 2, 1, 3], 0], [abc, abc, [2, 2, 0, 0], 0],
  [abc, abc, [2, 2, 0, 1], 1], [abc, abc, [0, 1, 3, 3], -1], [abc, abc, [undefined, undefined, 1], 1], [abc, abc, [1], -1],
  [abc, abc, [1, undefined, undefined, 2], -1], [abc, abc, [5], 1], [abc, abc, [5, 3], 1], [abc, abc, [0, 3, 5], -1],
  [abc, abc, [0, 3, 5, 3], -1], [abc, abc, [4, 3, 0, 0], 0], [[1, 2], abc, [0, 2, 0], 0], [abc, [1, 2], [0, undefined, 0, 2], 0]];
 for (const [a, b, args, want] of cases) expect(Buffer.from(a).compare(Buffer.from(b), ...args)).toBe(want);
});
test('instance compare validates every range argument with Node errors', () => {
 const b = Buffer.from([1, 2, 3]);
 const cases = [[[0, 4], range('targetEnd', '>= 0 && <= 3', 4)], [[0, 3, 0, 4], range('sourceEnd', '>= 0 && <= 3', 4)],
  [[-1], range('targetStart', '>= 0 && <= 9007199254740991', -1)], [[0, -1], range('targetEnd', '>= 0 && <= 3', -1)],
  [[1.5], range('targetStart', 'an integer', 1.5)], [['1'], type('targetStart', "type string ('1')")],
  [[0, 3, '0'], type('sourceStart', "type string ('0')")], [[null], type('targetStart', 'null')], [[NaN], range('targetStart', 'an integer', NaN)],
  [[0, Infinity], range('targetEnd', 'an integer', Infinity)], [[0, 3, 0, 3.5], range('sourceEnd', 'an integer', 3.5)],
  [[2 ** 53], range('targetStart', '>= 0 && <= 9007199254740991', '9_007_199_254_740_992')], [[4, 4], range('targetEnd', '>= 0 && <= 3', 4)],
  [[0, 3, 4, 4], range('sourceEnd', '>= 0 && <= 3', 4)], [[4, 5], range('targetEnd', '>= 0 && <= 3', 5)]];
 for (const [args, want] of cases) expect(caught(() => b.compare(b, ...args))).toEqual(want);
});
test('instance compare rejects non-Uint8Array targets with Node received descriptions', () => {
 class Foo {}
 const named = function named() {};
 const cases = [[null, 'null'], [undefined, 'undefined'], [[1], 'an instance of Array'], ['a', "type string ('a')"], [new Uint16Array([1]), 'an instance of Uint16Array'],
  [{}, 'an instance of Object'], [new DataView(new ArrayBuffer(1)), 'an instance of DataView'], [true, 'type boolean (true)'], [1n, 'type bigint (1n)'],
  [Symbol('s'), 'type symbol (Symbol(s))'], [named, 'function named'], ['x'.repeat(30), "type string ('xxxxxxxxxxxxxxxxxxxxxxxxx...')"],
  ["it's", 'type string ("it\'s")'], [-0, 'type number (-0)'], [1e21, 'type number (1e+21)'], [new Foo(), 'an instance of Foo'],
  [Object.create(null), '[Object: null prototype] {}']];
 for (const [value, want] of cases) expect(caught(() => Buffer.from([1]).compare(value))).toEqual(target(want));
});
test('static compare and equals keep their behaviour next to instance compare', () => {
 expect(Buffer.compare(Buffer.from([1]), Buffer.from([2]))).toBe(-1);
 expect(Buffer.from([1, 2]).equals(Buffer.from([1, 2]))).toBe(true);
 expect(Object.keys(Buffer)).toContain('compare');
 expect(Buffer.prototype.compare.length).toBe(5);
 expect(Buffer.prototype.write.length).toBe(4);
});
test('write resolves Node argument overloads', () => {
 const cases = [[[6, 0, 'héllo'], [6, [104, 195, 169, 108, 108, 111]]], [[6, 0, 'ab', 2, 'utf8'], [2, [0, 0, 97, 98, 0, 0]]],
  [[4, 0, 'abcd', 'hex'], [2, [171, 205, 0, 0]]], [[6, 0, 'abcdef', 1, 2, 'latin1'], [2, [0, 97, 98, 0, 0, 0]]],
  [[6, 0, 'abcdef', 1, 2], [2, [0, 97, 98, 0, 0, 0]]], [[4, 0, 'abcdef', 1], [3, [0, 97, 98, 99]]], [[4, 0, 'ab', undefined, 'hex'], [2, [97, 98, 0, 0]]],
  [[4, 0, 'abcd', 1, undefined, 'hex'], [2, [0, 171, 205, 0]]], [[4, 0, 'abcd', 0, 'HEX'], [2, [171, 205, 0, 0]]], [[4, 0, 'é', 'utf-8'], [2, [195, 169, 0, 0]]],
  [[4, 0, 'a', 0, 1, null], [1, [97, 0, 0, 0]]], [[4, 0, 'a', ''], [1, [97, 0, 0, 0]]], [[4, 0, 'a', 0, 1, ''], [1, [97, 0, 0, 0]]], [[4, 0, 'ab', -0], [2, [97, 98, 0, 0]]],
  [[4, 0, 'aé', 0, 'ASCII'], [2, [97, 233, 0, 0]]], [[4, 0, 'ab', 'Latin1'], [2, [97, 98, 0, 0]]], [[4, 0, 'ab', 'UTF8'], [2, [97, 98, 0, 0]]],
  [[4, 0, 'ab', 'Base64'], [1, [105, 0, 0, 0]]], [[4, 0, 'ab', 'UCS2'], [4, [97, 0, 98, 0]]], [[4, 0, 'ab', 'binary'], [2, [97, 98, 0, 0]]]];
 for (const [args, want] of cases) expect(written(...args)).toEqual(want);
});
test('utf8 write truncates on whole characters and replaces lone surrogates', () => {
 const cases = [[[3, 0, 'aé€'], [3, [97, 195, 169]]], [[4, 0, 'aé€'], [3, [97, 195, 169, 0]]], [[5, 0, 'aé€'], [3, [97, 195, 169, 0, 0]]],
  [[6, 0, 'aé€'], [6, [97, 195, 169, 226, 130, 172]]], [[4, 0, '😀'], [4, [240, 159, 152, 128]]], [[3, 0, '😀'], [0, [0, 0, 0]]],
  [[4, 7, 'a😀', 0], [1, [97, 7, 7, 7]]], [[4, 0, '\ud800'], [3, [239, 191, 189, 0]]], [[4, 0, 'a\udc00b'], [4, [97, 239, 191, 189]]], [[2, 0, '\ud800'], [0, [0, 0]]]];
 for (const [args, want] of cases) expect(written(...args)).toEqual(want);
});
test('single-byte and utf16 encodings write low bytes and whole code units', () => {
 const cases = [[[4, 0, 'éĀÿa', 'latin1'], [4, [233, 0, 255, 97]]], [[4, 0, 'éĀÿa', 'binary'], [4, [233, 0, 255, 97]]],
  [[2, 0, 'abc', 'latin1'], [2, [97, 98]]], [[4, 0, 'éĀÿa', 'ascii'], [4, [233, 0, 255, 97]]], [[3, 0, 'abcd', 'ascii'], [3, [97, 98, 99]]],
  [[4, 0, '\ud800', 'latin1'], [1, [0, 0, 0, 0]]], [[4, 0, '\ud800x', 'utf16le'], [4, [0, 216, 120, 0]]], [[5, 0, 'ab€', 'utf16le'], [4, [97, 0, 98, 0, 0]]],
  [[4, 0, 'ab€', 'ucs2'], [4, [97, 0, 98, 0]]], [[4, 0, 'ab', 'ucs-2'], [4, [97, 0, 98, 0]]], [[4, 0, 'ab', 'utf-16le'], [4, [97, 0, 98, 0]]],
  [[3, 0, 'abc', 1, 'utf16le'], [2, [0, 97, 0]]]];
 for (const [args, want] of cases) expect(written(...args)).toEqual(want);
});
test('hex write stops at the first invalid or incomplete pair', () => {
 const cases = [[[4, 0, '0a0B', 'hex'], [2, [10, 11, 0, 0]]], [[4, 0, 'abc', 'hex'], [1, [171, 0, 0, 0]]], [[4, 0, 'abzz12', 'hex'], [1, [171, 0, 0, 0]]],
  [[4, 0, 'zz12', 'hex'], [0, [0, 0, 0, 0]]], [[2, 0, '010203', 'hex'], [2, [1, 2]]], [[4, 9, '', 'hex'], [0, [9, 9, 9, 9]]], [[4, 0, 'a', 'hex'], [0, [0, 0, 0, 0]]],
  [[4, 0, '0g', 'hex'], [0, [0, 0, 0, 0]]]];
 for (const [args, want] of cases) expect(written(...args)).toEqual(want);
});
test('base64 write accepts both alphabets, skips noise and stops at padding', () => {
 const hello = [5, [104, 101, 108, 108, 111, 0]];
 const cases = [[['aGVsbG8=', 'base64'], hello], [['aGVsbG8', 'base64url'], hello], [['aGV sbG8=', 'base64'], hello], [['-_-_', 'base64'], [3, [251, 255, 191, 0, 0, 0]]],
  [['+/+/', 'base64url'], [3, [251, 255, 191, 0, 0, 0]]], [['YQ==YQ==', 'base64'], [1, [97, 0, 0, 0, 0, 0]]], [['', 'base64'], [0, [0, 0, 0, 0, 0, 0]]],
  [['YWJj\nZGVm', 'base64'], [6, [97, 98, 99, 100, 101, 102]]], [['Y', 'base64'], [0, [0, 0, 0, 0, 0, 0]]], [['YW', 'base64'], [1, [97, 0, 0, 0, 0, 0]]],
  [['Y!Q', 'base64'], [1, [97, 0, 0, 0, 0, 0]]], [['YWJj=ZGVm', 'base64'], [3, [97, 98, 99, 0, 0, 0]]], [['YWJjZGU', 'base64url'], [5, [97, 98, 99, 100, 101, 0]]],
  [['YWéJj', 'base64'], [3, [97, 98, 99, 0, 0, 0]]], [['=YWJj', 'base64'], [0, [0, 0, 0, 0, 0, 0]]], [['YWJ=j', 'base64'], [2, [97, 98, 0, 0, 0, 0]]]];
 for (const [args, want] of cases) expect(written(6, 0, ...args)).toEqual(want);
 expect(written(3, 0, 'aGVsbG8=', 'base64')).toEqual([3, [104, 101, 108]]);
});
test('write clamps to the remaining bytes and validates offset and length', () => {
 const unknown = (name) => ['TypeError', 'ERR_UNKNOWN_ENCODING', `Unknown encoding: ${name}`];
 const notString = ['TypeError', 'ERR_INVALID_ARG_TYPE', 'argument must be a string'];
 const cases = [[[4, 0, 'abc', 4], [0, [0, 0, 0, 0]]], [[4, 0, 'abc', 4, 0], [0, [0, 0, 0, 0]]], [[4, 0, 'abc', 3, 5], range('length', '>= 0 && <= 4', 5)],
  [[4, 0, 'abc', 0, 0], [0, [0, 0, 0, 0]]], [[4, 0, 'abc', 2, 1], [1, [0, 0, 97, 0]]], [[0, 0, 'abc'], [0, []]], [[0, 0, 'abc', 0], [0, []]],
  [[4, 0, 'a', 5], range('offset', '>= 0 && <= 4', 5)], [[4, 0, 'a', -1], range('offset', '>= 0 && <= 4', -1)], [[4, 0, 'a', 1.5], range('offset', 'an integer', 1.5)],
  [[4, 0, 'a', '1', 1], type('offset', "type string ('1')")], [[4, 0, 'a', null], type('offset', 'null')], [[4, 0, 'a', NaN], range('offset', 'an integer', NaN)],
  [[4, 0, 'a', 0, 5], range('length', '>= 0 && <= 4', 5)], [[4, 0, 'a', 0, -1], range('length', '>= 0 && <= 4', -1)], [[4, 0, 'a', 0, 1.5], range('length', 'an integer', 1.5)],
  [[4, 0, 'a', 0, null], type('length', 'null')], [[4, 0, 'a', true], type('offset', 'type boolean (true)')], [[4, 0, 'a', 2 ** 32], range('offset', '>= 0 && <= 4', 4294967296)],
  [[4, 0, 'a', -(2 ** 32) - 1], range('offset', '>= 0 && <= 4', '-4_294_967_297')], [[4, 0, 'a', 1e21], range('offset', '>= 0 && <= 4', '1e_+21')],
  [[4, 0, 'a', 'nope'], unknown('nope')], [[4, 0, 'a', 0, 'nope'], unknown('nope')], [[4, 0, 'a', 0, 1, 'nope'], unknown('nope')], [[4, 0, 'a', 0, 1, 5], unknown(5)],
  [[4, 0, 5], notString], [[4, 0, null, 0], notString], [[4, 0, undefined], notString], [[4, 0, { toString() { return 'x'; } }, 'latin1'], notString], [[4, 0, 5, 0, 'hex'], notString]];
 for (const [args, want] of cases) expect(written(...args)).toEqual(want);
});
"##;
#[test]
fn buffer_instance_compare_and_string_write_match_node() {
    let p = Project::new("buffer-compare-write-408-409");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("buffer.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 12, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
