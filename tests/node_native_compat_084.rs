//! Native Node API regressions; the same assertions first run under Node as controls.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::process::Command;

fn check(name: &str, source: &str) {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"node-native-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    let fixture = p.file("compat.test.cjs", source);
    let output = Command::new("node")
        .arg("-e")
        .arg("global.test = (_name, fn) => fn(); require(process.argv[1]);")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Node control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn ip_classification_node_control() {
    check(
        "node-ip-084",
        r#"
const assert = require('node:assert/strict');
const { isIP, isIPv4, isIPv6 } = require('node:net');
test('classifies IP literals and rejects invalid hosts', () => {
  const values = [
    ['127.0.0.1', 4], ['0.0.0.0', 4], ['255.255.255.255', 4],
    ['::', 6], ['::1', 6], ['::ffff:192.168.1.1', 6], ['fe80::1%lo0', 6],
    ['::1%a-.:0', 6], ['', 0], ['localhost', 0], ['127.1', 0],
    ['01.2.3.4', 0], ['256.0.0.1', 0], ['::1%', 0], ['::1%a b', 0],
    ['::1%a%b', 0], ['127.0.0.1%a', 0], ['[::1]', 0], ['::ffff:192.168.01.1', 0],
    [null, 0], [undefined, 0], [123, 0], [{toString() { return '127.0.0.1'; }}, 4],
  ];
  for (const [value, version] of values) {
    assert.equal(isIP(value), version);
    assert.equal(isIPv4(value), version === 4);
    assert.equal(isIPv6(value), version === 6);
  }
  assert.throws(() => isIP(Symbol('ip')), TypeError);
});
"#,
    );
}

#[test]
fn gzip_node_control() {
    check(
        "node-gzip-084",
        r#"
const assert = require('node:assert/strict');
const { gzipSync, gunzipSync } = require('node:zlib');
test('produces real gzip bytes and validates compressed input', () => {
  const source = Buffer.from('hello \u{1f680} hello');
  const nodeGzip = Buffer.from([31,139,8,0,0,0,0,0,0,19,203,203,79,73,85,72,206,207,43,41,202,207,1,0,5,234,163,62,12,0,0,0]);
  assert.equal(gunzipSync(nodeGzip).toString(), 'node control');
  const gzip = gzipSync(source);
  assert.equal(Buffer.isBuffer(gzip), true);
  assert.deepEqual([...gzip.subarray(0, 3)], [31, 139, 8]);
  assert.deepEqual([...gunzipSync(gzip)], [...source]);
  assert.equal(gunzipSync(gzipSync('')).length, 0);
  assert.equal(gunzipSync(gzipSync('hello', null), null).toString(), 'hello');
  assert.equal(gunzipSync(Buffer.concat([gzip, Buffer.from([0, 0])])).toString(), source.toString());
  assert.equal(gunzipSync(Buffer.concat([gzip, Buffer.from([0, 1])])).toString(), source.toString());
  assert.throws(() => gunzipSync(Buffer.concat([gzip, Buffer.from([1])])));
  assert.throws(() => gunzipSync(Buffer.alloc(0)));
  assert.equal(gunzipSync(gzipSync('hello', {level: 0})).toString(), 'hello');
  assert.equal(gunzipSync(gzipSync('hello', {level: 9})).toString(), 'hello');
  const joined = Buffer.concat([gzipSync('one'), gzipSync('two')]);
  assert.equal(gunzipSync(joined).toString(), 'onetwo');
  const padded = Buffer.concat([Buffer.from([1]), gzip, Buffer.from([2])]);
  assert.deepEqual([...gunzipSync(padded.subarray(1, padded.length - 1))], [...source]);
  assert.throws(() => gunzipSync(Buffer.from('plain bytes')));
  assert.throws(() => gunzipSync(gzip.subarray(0, gzip.length - 1)));
  const bad = Buffer.from(gzip); bad[bad.length - 8] ^= 1;
  assert.throws(() => gunzipSync(bad));
  assert.throws(() => gzipSync(123), TypeError);
  assert.throws(() => gzipSync('x', {level: 10}), RangeError);
});
"#,
    );
}

#[test]
fn v8_serialization_node_control() {
    check(
        "node-v8-084",
        r#"
const assert = require('node:assert/strict');
const { serialize, deserialize } = require('node:v8');
test('V8 serialization preserves structure and rejects malformed bytes', () => {
  const source = { map: new Map([['x', new Set([1, 2])]]),
    view: new Uint16Array([0, 65535, 1234]), date: new Date(12345),
    regex: /foo/gi, big: 12345678901234567890n };
  source.self = source;
  const nodeBytes = Buffer.from([255,15,111,34,6,103,111,108,100,101,110,59,34,4,110,111,100,101,73,84,58,2,123,1]);
  assert.equal(deserialize(nodeBytes).golden.get('node'), 42);
  const encoded = serialize(source);
  assert.equal(Buffer.isBuffer(encoded), true);
  const restored = deserialize(encoded);
  assert.equal(restored.self, restored);
  assert.equal(restored.map instanceof Map, true);
  assert.equal(restored.map.get('x') instanceof Set, true);
  assert.deepEqual([...restored.map.get('x')], [1, 2]);
  assert.equal(restored.view instanceof Uint16Array, true);
  assert.deepEqual([...restored.view], [0, 65535, 1234]);
  assert.equal(restored.date.getTime(), 12345);
  assert.equal(restored.regex.source, 'foo');
  assert.equal(restored.regex.flags, 'gi');
  assert.equal(restored.big, source.big);
  for (const value of [null, undefined, false, 42, 'hello']) {
    assert.equal(deserialize(serialize(value)), value);
  }
  const padded = Buffer.concat([Buffer.from([0]), encoded, Buffer.from([0])]);
  assert.equal(deserialize(padded.subarray(1, padded.length - 1)).self.self.map.get('x').has(2), true);
  assert.throws(() => serialize(() => {}));
  assert.throws(() => serialize(Symbol('x')));
  assert.throws(() => deserialize(Buffer.alloc(0)));
  assert.throws(() => deserialize(Buffer.from([1, 2, 3])));
  assert.throws(() => deserialize(encoded.subarray(0, encoded.length - 1)));
  assert.throws(() => deserialize('not bytes'), TypeError);
});
"#,
    );
}
