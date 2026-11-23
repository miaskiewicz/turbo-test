//! turbo-test#462: `crypto.subtle` under the DOM env must support more than SHA-256 digest —
//! `importKey` + `sign` / `verify` for HMAC, and AES-GCM `encrypt` / `decrypt`. The #459
//! bug-6 reproducer (HMAC-SHA-256 sign producing a 32-byte MAC) drives the first assertion.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn subtle_crypto_hmac_and_aes_gcm_round_trip() {
    let p = Project::new("crypto-subtle-462");
    p.file("package.json", r#"{"name":"crypto-subtle-462"}"#);
    p.file(
        "vitest.config.mjs",
        "export default { test: { environment: 'happy-dom', globals: true } }\n",
    );
    p.file(
        "cases/subtle.test.mjs",
        r#"
import { expect, test } from 'vitest';

const enc = new TextEncoder();

test('HMAC importKey + sign yields a 32-byte SHA-256 MAC (#459 bug 6)', async () => {
  const key = await crypto.subtle.importKey(
    'raw', enc.encode('secret'),
    { name: 'HMAC', hash: 'SHA-256' },
    false, ['sign', 'verify'],
  );
  const sig = await crypto.subtle.sign('HMAC', key, enc.encode('hello'));
  expect(sig).toBeInstanceOf(ArrayBuffer);
  expect(sig.byteLength).toBe(32);
});

test('HMAC verify accepts a valid signature and rejects a tampered one', async () => {
  const key = await crypto.subtle.importKey(
    'raw', enc.encode('k'),
    { name: 'HMAC', hash: 'SHA-256' },
    false, ['sign', 'verify'],
  );
  const data = enc.encode('message');
  const sig = await crypto.subtle.sign('HMAC', key, data);
  expect(await crypto.subtle.verify('HMAC', key, sig, data)).toBe(true);
  const bad = new Uint8Array(sig).slice();
  bad[0] ^= 0xff;
  expect(await crypto.subtle.verify('HMAC', key, bad, data)).toBe(false);
});

test('SHA-1 HMAC produces a 20-byte MAC', async () => {
  const key = await crypto.subtle.importKey(
    'raw', enc.encode('k'),
    { name: 'HMAC', hash: 'SHA-1' },
    false, ['sign'],
  );
  const sig = await crypto.subtle.sign('HMAC', key, enc.encode('x'));
  expect(sig.byteLength).toBe(20);
});

test('SHA-256 digest still works', async () => {
  const d = await crypto.subtle.digest('SHA-256', enc.encode('hello'));
  expect(new Uint8Array(d).length).toBe(32);
});

test('AES-GCM encrypt / decrypt round-trips with a 256-bit key', async () => {
  const raw = crypto.getRandomValues(new Uint8Array(32));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await crypto.subtle.importKey(
    'raw', raw, { name: 'AES-GCM' }, false, ['encrypt', 'decrypt'],
  );
  const plain = enc.encode('top secret');
  const cipher = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plain);
  expect(cipher).toBeInstanceOf(ArrayBuffer);
  expect(cipher.byteLength).toBeGreaterThan(plain.byteLength); // tag appended
  const back = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, cipher);
  expect(new TextDecoder().decode(back)).toBe('top secret');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numPassedTests"], 5, "{}", r.all());
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
}
