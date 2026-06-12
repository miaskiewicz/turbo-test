# M0 — Snapshot-isolation spike: findings (VALIDATED)

Run: `cargo run --release --bin m0-spike` (8-core M-series, v8 149.3.0).
Real baseline: stock Vitest 4.1.8 on `../ui-design-components/src/utils` (10 files, 145 tests).

## The decision M0 had to make

Is **fresh-context-per-file** (the spec's choice — full isolation = full compatibility)
fast enough to beat Vitest, or must we sacrifice isolation (reuse+reset) to compete?

**Answer: fresh-context is overwhelmingly fast enough. Keep full isolation.**

## Per-file setup cost by strategy (µs, all reuse one isolate)

```
   funcs   blob(KB)      rebuild  reuse+reset     snapshot       shared
       0        481        140.4          6.5        136.6          2.0
     500        826        760.3          6.6        225.5          2.0
    2000       1399       2952.1          6.4        382.8          2.1
    8000       3807       5753.2          6.5       1188.2          2.3
throughput @ 2000-func snapshot (8 isolates): ~6300 contexts/sec
isolate creation (once per worker, NOT per file): ~0.35 ms
```

- `rebuild` = fresh context + re-evaluate framework + test (no snapshot)
- `snapshot` = fresh context from blob (framework baked) + test — **full isolation**
- `reuse+reset` = one context reused, globals deleted between files — **leaky**
- `shared` = one context, no reset — **no isolation** (absolute floor)

## The number that matters: vs what stock Vitest actually does per file

Vitest's per-file cost is dominated by **environment construction** — the exact thing
snapshot-isolation replaces. Measured (summed CPU work ÷ file count):

| path | env setup / file | snapshot is |
|---|---|---|
| jsdom + isolate=true (default) | **465 ms** | **1215× cheaper** |
| turbo-dom + shared (config.ci, fastest) | **32 ms** | **84× cheaper** |
| turbo-test snapshot fresh-context | **0.38 ms** | — |

Full stock-vitest per-file breakdown (jsdom run, 10 files):
`transform 64ms · setup 179ms · import 53ms · environment 465ms · tests 8ms` per file.
**Test logic is ~8ms; everything else (~760ms) is substrate** — env, framework import,
transform. That substrate is precisely what this design removes.

## Isolation question — settled with evidence

`reuse+reset` (6.4µs) is ~60× faster than `snapshot` fresh-context (383µs) in the
microbench. But that 377µs delta is **noise** beside the 32,000µs of env setup snapshot
deletes. And reuse+reset only resets *globals we remember* — not the module registry,
prototype patches, or mocked `Date`/timers, all of which leak between files and break
compatibility (the product's whole thesis, §2).

=> **Keep fresh-context-per-file.** Full isolation costs ~0.38ms/file; it is free in
practice and preserves compatibility.

## Hard constraint surfaced early (the M3 risk, seen at M0)

`Context::new` from a snapshot **deserializes the whole baked heap every time**, so per-file
cost scales with blob size: 137µs (481KB) → 383µs (1.4MB) → 1.19ms (3.8MB). Still far under
the 32ms env baseline, so there is comfortable headroom — but **the snapshot must be kept
lean.** Bake the framework + turbo-dom, nothing more; lazy-load the rest. The §8 guardrail
("snapshot instantiation must not regress as the snapshot grows") is real and tracked from
here on.

## Verdict

**M0 PASS.** Snapshot fresh-context is 84–1215× cheaper than stock Vitest's per-file
environment construction, with full isolation retained. The spec premise holds; the only
correction is that context creation is sub-millisecond, not "microseconds," and scales with
snapshot size — manageable by keeping the snapshot lean. **Proceed to M1.**
