//! turbo-test#463: `new Response(ReadableStream).text()` and `.json()` must consume the stream
//! body instead of stringifying it (`'[object Object]'`). Same bug on `Request`. Reproducer shape
//! follows issue #459 — React's `renderToReadableStream` wrapped in a `Response`.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn response_and_request_text_json_consume_readable_stream_bodies() {
    let p = Project::new("response-stream-body-463");
    p.file("package.json", r#"{"name":"response-stream-body-463"}"#);
    p.file(
        "vitest.config.mjs",
        "export default { test: { environment: 'node', globals: true } }\n",
    );
    p.file(
        "cases/stream.test.mjs",
        r#"
import { expect, test } from 'vitest';

const streamOf = (chunks) => new ReadableStream({
  start(controller) {
    for (const c of chunks) controller.enqueue(typeof c === 'string' ? new TextEncoder().encode(c) : c);
    controller.close();
  },
});

test('Response.text() reads a ReadableStream body', async () => {
  const res = new Response(streamOf(['hello, ', 'world']));
  expect(await res.text()).toBe('hello, world');
});

test('Response.json() parses a ReadableStream JSON body', async () => {
  const res = new Response(streamOf(['{"ok":', 'true,"n":42}']));
  expect(await res.json()).toEqual({ ok: true, n: 42 });
});

test('Request.text() reads a ReadableStream body', async () => {
  const req = new Request('https://example.test/', { method: 'POST', body: streamOf(['a', 'b', 'c']) });
  expect(await req.text()).toBe('abc');
});

test('Response.text() rejects when the stream errors', async () => {
  const res = new Response(new ReadableStream({
    start(controller) { controller.error(new Error('boom')); },
  }));
  await expect(res.text()).rejects.toThrow(/boom/);
});

test('string bodies still go through the fast path', async () => {
  const res = new Response('plain string');
  expect(await res.text()).toBe('plain string');
  const res2 = new Response('{"x":1}');
  expect(await res2.json()).toEqual({ x: 1 });
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numPassedTests"], 5, "{}", r.all());
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
}
