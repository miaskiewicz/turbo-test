import { describe, test, expect } from 'vitest';

// async.node (tests/napi_support/async.c) is compiled by tests/napi_host.rs next to this file.
// The expected values are Node 26's: the same spec passes against Node's own Node-API (but the
// host-only NULL-handle checks, on which Node aborts).
const a = require('./async.node');

const OK = 0;
const INVALID_ARG = 1;
const OBJECT_EXPECTED = 2;
const GENERIC_FAILURE = 9;
const PENDING_EXCEPTION = 10;
const CANCELLED = 11;
const CALLBACK_SCOPE_MISMATCH = 14;
const QUEUE_FULL = 15;
const CLOSING = 16;
const UV_EBUSY = -16;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('async work', () => {
  test('execute runs on a pool thread, complete on the JS thread', async () => {
    const results = await Promise.all([a.work(1), a.work(2), a.work(21)]);
    expect(results).toEqual([[OK, 2, 1], [OK, 4, 1], [OK, 42, 1]]);
  });

  test('queued work can be cancelled, running work cannot', async () => {
    const done = a.blockPool();
    // the four pool threads are busy
    while (a.started() < 4) await sleep(1);
    expect(a.queueAndCancel()).toEqual([OK, GENERIC_FAILURE, GENERIC_FAILURE, UV_EBUSY]);
    // the blockers complete with napi_ok, the cancelled work with napi_cancelled
    expect(await done).toEqual([OK, OK, OK, OK, CANCELLED]);
  });

  test('an exception left by complete is uncaught', async () => {
    const seen: string[] = [];
    const onUncaught = (e: Error) => seen.push(e.message);
    (globalThis as any).process.on('uncaughtException', onUncaught);
    try {
      expect(await a.workThrows()).toBe('resolved before the throw');
      await sleep(1);
    } finally {
      (globalThis as any).process.off('uncaughtException', onUncaught);
    }
  });

  test('argument checks', () => {
    // a non-object resource is ToObject'd; a work with no complete callback runs
    expect(a.workChecks()).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG, INVALID_ARG, OK, OK]);
  });
});

describe('threadsafe functions', () => {
  test('calls from another thread run in order on the JS thread; blocking calls wait for space', async () => {
    const seen: number[] = [];
    const r = await a.tsfnThreads((i: number) => seen.push(i), 6);
    expect(seen).toEqual([0, 1, 2, 3, 4, 5]);
    // six napi_ok calls, the release, no NULL-env call, the finalize data
    expect(r.slice(0, 6)).toEqual(Array(6).fill(OK));
    expect(r.slice(15)).toEqual([OK, 0, 1]);
  });

  test('the default call_js calls the function with no arguments', async () => {
    const calls: unknown[][] = [];
    const r = await a.tsfnDefault((...args: unknown[]) => calls.push(args));
    expect(calls).toEqual([[]]);
    expect(r.slice(0, 3)).toEqual([OK, OK, OK]);
  });

  test('an aborted call through the default call_js never calls the function', async () => {
    const calls: unknown[][] = [];
    const r = await a.tsfnDefaultAborted((...args: unknown[]) => calls.push(args));
    expect(r.slice(0, 3)).toEqual([OK, OK, OK]);
    expect(calls).toEqual([]);
  });

  test('a full queue, acquire, context, ref / unref, abort', async () => {
    const r = await a.tsfnLifecycle();
    // ... and with no holder left a call is napi_invalid_arg
    expect(r.slice(0, 13)).toEqual([OK, OK, QUEUE_FULL, OK, OK, 1, OK, OK, OK, CLOSING, CLOSING,
      INVALID_ARG, INVALID_ARG]);
    // the call still queued at the abort reaches call_js with a NULL env
    expect(r[16]).toBe(1);
  });

  test('a release with no holder left is napi_invalid_arg', async () => {
    const [st, done] = a.tsfnRelease();
    expect(st).toEqual([OK, OK, OK, INVALID_ARG]);
    const r = await done;
    expect(r[16]).toBe(0);
  });

  test('finalized while threads still hold it: freed when the last one lets go', async () => {
    const r = await a.tsfnHeld();
    expect(r.slice(0, 2)).toEqual([OK, OK]);
    expect(a.tsfnHeldLetGo()).toEqual([CLOSING, OK]);
  });

  test('an exception left by call_js is dropped (module API version 8)', async () => {
    expect(await a.tsfnThrows()).toBe('called');
    await sleep(1);
  });

  test('argument checks', () => {
    expect(a.tsfnChecks(() => {})).toEqual([INVALID_ARG, INVALID_ARG, INVALID_ARG, INVALID_ARG,
      INVALID_ARG]);
  });

  // host-only: in Node these calls come after the threadsafe function was freed
  test('host-only: holders letting go after the finalization free it', async () => {
    for (const release of [false, true]) {
      const r = await a.tsfnHeldLate();
      expect(r.slice(0, 2)).toEqual([OK, OK]);
      await sleep(0);
      expect(a.tsfnLateLetGo(release)).toEqual([CLOSING, release ? OK : CLOSING]);
    }
  });

  // host-only: Node aborts on a NULL threadsafe function
  test('host-only: a NULL threadsafe function is napi_invalid_arg', () => {
    expect(a.nullHandles()).toEqual(Array(6).fill(INVALID_ARG));
  });
});

describe('async contexts, napi_make_callback, callback scopes, the uv loop', () => {
  test('napi_make_callback', () => {
    const recv = { tag: 'recv' };
    const [st, out, exc] = a.makeCallback(recv, function (this: any, x: number) {
      return [this.tag, x * 2];
    }, 21);
    expect([st, out, exc]).toEqual([OK, ['recv', 42], undefined]);
    expect(a.makeCallback(recv, 5, 1)).toEqual([INVALID_ARG, undefined, undefined]);
    const [st2, , exc2] = a.makeCallback(recv, () => { throw new Error('in callback'); }, 1);
    expect(st2).toBe(PENDING_EXCEPTION);
    expect(exc2.message).toBe('in callback');
    // the receiver is ToObject'd: undefined throws
    const [st3, , exc3] = a.makeCallback(undefined, () => 1, 1);
    expect(st3).toBe(OBJECT_EXPECTED);
    expect(exc3).toBeInstanceOf(TypeError);
  });

  test('argument checks, callback scopes', () => {
    expect(a.asyncChecks(() => 1)).toEqual([INVALID_ARG, INVALID_ARG, OK, OK, INVALID_ARG,
      INVALID_ARG, OK, INVALID_ARG, OK, OK, CALLBACK_SCOPE_MISMATCH, INVALID_ARG, INVALID_ARG,
      OK]);
  });

  test('napi_get_uv_event_loop', () => {
    expect(a.uvLoop()).toEqual([OK, 1, INVALID_ARG]);
  });
});
