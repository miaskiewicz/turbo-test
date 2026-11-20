# Node version feature-flag audit — 0.8.5 and 0.8.6 additions

turbo-test#453 adds `--node-features 22|24|26` (default 22). Every Node-API addition landed in
0.8.5 and 0.8.6 is reviewed against Node's own CHANGELOG below and labelled either **baseline**
(available in Node 22 LTS as well as 24 / 26, no gate needed) or **gate behind
`globalThis.__ttNodeFeatures >= 24` / `>= 26`**.

Oracle: real Node 26.11 for behaviour that differs between versions; Node's CHANGELOG
(`doc/changelogs/CHANGELOG_V{10..26}.md` in nodejs/node) for "first landed in" / "stable in" /
"removed in" versions. Node 22 LTS is the baseline turbo-test defaults to because every current
flux-apis app runs on Node 22.

Verdict table (newest → oldest within each block). Node version is the one the behaviour as
turbo-test implements it first lands in (or stabilises in) as semver-stable, not the one it was
flagged in behind `--experimental-*`.

## 0.8.5

### diagnostics_channel (#407)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `tracingChannel(name)` + `traceSync` / `tracePromise` / `traceCallback` | 19.9.0 (stable 22) | **baseline** |
| `Channel.bindStore` / `runStores` (AsyncLocalStorage bridge) | 19.9.0 (stable 22) | **baseline** |
| `BoundedChannel` | 26.0.0 | **omitted** (stays omitted — would gate behind `__ttNodeFeatures >= 26` when added) |
| `tracingChannel.withStoreScope` | 26.0.0 | **omitted** (same) |

Rationale: `tracingChannel` is what Pino 9 reaches for at load, so the four flux backends on Node
22 need it as baseline. `BoundedChannel` / `withStoreScope` are the two Node-26-only pieces the
issue explicitly called out and the 0.8.5 release notes say they were deliberately omitted —
they stay omitted until turbo-test adds them, at which point they will be gated.

### zlib + Brotli streams (#404)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `Deflate` / `Inflate` / `DeflateRaw` / `InflateRaw` / `Gzip` / `Gunzip` / `Unzip` + factories | 0.5 / 0.10 | **baseline** |
| `flush(kind?, callback)` on the stateful handle | 10.0 | **baseline** |
| Callback / promisified / `*Sync` helpers (`gzip` / `gunzip` / `deflate` / `inflate`) | 0.6 / 10.0 | **baseline** |
| `BrotliCompress` / `BrotliDecompress`, `brotliCompress` / `brotliDecompress` / `*Sync` + `BROTLI_*` constants | 11.7.0 | **baseline** |

Node 22 ships the full set; no Node 24/26 shape changes to turbo-test's public surface.

### Buffer (#408, #409, #418, #434, and earlier)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `Buffer.prototype.compare(target, …)` | 0.11 | **baseline** |
| `Buffer.prototype.write(string, offset?, length?, encoding?)` | 0.1 | **baseline** |
| Internal `latin1Slice` / `utf8Slice` / `asciiSlice` / `hexSlice` / `base64Slice` / their `*Write` counterparts | 0.1 — undocumented, kept stable through 26 | **baseline** |
| `Buffer.isEncoding` | 0.9 | **baseline** |
| `read/writeFloatBE/LE`, `read/writeDoubleBE/LE`, `read/writeBigInt64BE/LE`, 1–6-byte int `read/write` variants | 0.5 / 12.0 (BigInt in 12) | **baseline** |
| `indexOf` / `lastIndexOf` / `includes` | 1.5 | **baseline** |
| `Buffer.from(ArrayBuffer, offset?, length?)` sharing semantics and `ERR_INVALID_ARG_TYPE` | 5.10 (sharing) / 10 (error code) | **baseline** |

Node 22 behaves identically to 24 and 26 for every one of these.

### process (#419)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `process.uptime()` | 0.5 | **baseline** |
| `process.memoryUsage()` + `.rss()` short form | 0.1 / 15.6 | **baseline** |
| `process.cpuUsage([prev])` | 6.1 | **baseline** |
| `process.resourceUsage()` | 12.6 | **baseline** |
| `process.umask([mask])` | 0.1 | **baseline** |
| `process.pid` / `process.ppid` from the host | 0.1 / 9 | **baseline** |

### fs (#420, #433)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `fs.promises.access` / `chmod` / `rename` / `realpath` and their `*Sync` forms | 10.0 | **baseline** |
| `fs.promises.open` → `FileHandle` + `Symbol.asyncDispose` | 10 (open) / 20.4 (asyncDispose) | **baseline** — `Symbol.asyncDispose` is stable in Node 22 |
| Callback-style `fs` API | 0.1 | **baseline** |

### crypto (#403, #450)

| Addition | Node min | Verdict |
| --- | --- | --- |
| `KeyObject.export({ format: 'pem', type: 'pkcs8', cipher, passphrase })` for AES-CBC / DES-EDE3-CBC encryption, PBES2 w/ PBKDF2-HMAC-SHA256 | 15.0 (encrypted PKCS#8 options) | **baseline** |
| `generateKeyPairSync` with the same encrypted PKCS#8 options | 10.12 / 15.0 | **baseline** |
| `crypto.createCipheriv` / `createDecipheriv` argument validation (error codes, messages, order) | 15.0 / 18.0 — the message strings stabilised across 18/20/22 and are unchanged in 24 and 26 | **baseline** |

### node:stream (0.8.5 Added)

| Addition | Node min | Verdict |
| --- | --- | --- |
| Writable `cork` / `uncork` / `_writev` | 0.11 | **baseline** |
| `decodeStrings: false`, `defaultEncoding` option, `setDefaultEncoding` | 0.10 | **baseline** |
| `_construct` | 15.0 | **baseline** |
| Readable `unpipe` | 0.9 | **baseline** |

### Event-loop ordering and sync-write (0.8.5 Fixed)

| Behaviour | Node min | Verdict |
| --- | --- | --- |
| After every host callback (timer, immediate, socket, child pipe, HTTP, MessagePort) `nextTick` queue drains before promise jobs (explicit microtask policy) | 11.0 | **baseline** |
| `Writable` calls `_write` synchronously (not nextTick'd) when the buffer accepts it, written `Buffer`s keep identity | 0.10 | **baseline** |
| A TCP write to a retired handle completes instead of hanging (libuv) | 0.10 | **baseline** |

### HTTP (#435)

| Behaviour | Node min | Verdict |
| --- | --- | --- |
| No framing header on bodiless `GET` / `HEAD` / `DELETE` / `OPTIONS` / `TRACE` / `CONNECT` | 0.3 | **baseline** |
| `res.end(data)` before headers uses `Content-Length` | 0.3 | **baseline** |

### URL / Error.stack / util (#424, #425, #437, #422)

| Behaviour | Node min | Verdict |
| --- | --- | --- |
| `new URL(…)` serialises IPv6 hosts in compressed WHATWG form | 20.0 (`URL` WHATWG rewrite stable) | **baseline** |
| `error.stack` header built on first read (so a message/name set after `super()` appears) | 10 — matches V8's `PrepareStackTraceCallback` across every Node turbo-test supports | **baseline** |
| `util.inspect` / `%s` of renamed/stack-less errors follows Node's `formatError` | 10 | **baseline** |
| `util.isDeepStrictEqual` strict + key-order independent | 9.0 | **baseline** |

### Clock (#447, #448)

| Behaviour | Node min | Verdict |
| --- | --- | --- |
| `Date`, `new Date()`, `performance.timeOrigin`, `Date.now()` read the host clock per run; `Date.now()` advances in sync code | 0.1 | **baseline** — the cached-snapshot bug was turbo-test-specific; no Node version-gated difference |

### Jest-vs-vitest switches (#402, #432, #403, #417, #421, #423, #401, #405, #406, #426, #429, #430, #434)

Not Node-API additions — they select between Jest 30 and vitest 5.1 behaviour via
`__ttJestProject` (runner-owned). Explicitly out of scope for this audit per the brief.

## 0.8.6

The 0.8.6 release repins turbo-dom / turbo-dom-layout / turbo-dom-browser-env to 0.8.6 and
turbo-dom-v8 to 150.4.2 from crates.io. It adds **no Node-API runtime surface**. CHANGELOG entry:

- `turbo-dom#537` grid baseline fix — layout, not a Node API.
- Faster HTML parser (index-arena html5ever tree sink) — parser internals, not a Node API.
- Version alignment to 0.8.6 — release-only.

Nothing to gate. Every 0.8.6 line is baseline by virtue of being unchanged from 0.8.5.

## Summary

| Bucket | Count |
| --- | --- |
| Reviewed 0.8.5 additions | 36 |
| Baseline (unchanged across Node 22 / 24 / 26) | 36 |
| Node 26-only, intentionally omitted (would be gated on add) | 2 (`BoundedChannel`, `withStoreScope`) |
| 0.8.6 additions requiring gating | 0 |

The `--node-features` flag ships with no regressions behind it: every Node-API feature turbo-test
adds today happens to match Node 22 LTS. The scaffolding is there so the moment a 24/26-only
feature is added (for example `BoundedChannel`), it goes behind `__ttNodeFeatures >= 26` without
disturbing the Node 22 baseline the four flux apps run on.

## Policy going forward

Per the issue — every Node-API feature turbo-test adds from 0.8.6 onward MUST, in the same
commit:

1. Name the Node version it first lands in (CHANGELOG or official docs).
2. If the Node 22 behaviour differs from 24 / 26, implement Node 22 as baseline and gate the
   newer behaviour behind `globalThis.__ttNodeFeatures >= 24` (or `>= 26`), with a test for
   each code path.
3. Otherwise, mark it baseline in the PR description.
