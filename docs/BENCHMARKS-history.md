# Benchmarks — historical snapshot (pre-consolidation)

These are the numbers the README carried before the 0.6.0 turbo-dom consolidation. They
were measured on **turbo-test 0.2.x** and have **not** been re-run on 0.6.0 (the re-run
for all four repos is planned for after the release, on a quiet machine). Read them as
history, not as a statement about 0.6.0.

## Full-suite wall time vs vitest (turbo-test 0.2.16, README refresh 2026-06-18)

Machine: Apple M-series workstation, 8 workers, same session for every column; each cell is
the median of repeated full-suite runs. turbo-test ran its all-Rust DOM (then opt-in via
`TURBO_RUST_DOM`; it is the only DOM now). The workstation was busy (a background VM held
~20% CPU, variance ±30%+ per run), so the seconds were marked tentative at the time; the
ratios are the more useful figure.

| Suite | Tests | vitest + jsdom | vitest + turbo-dom | turbo-test | vs jsdom | vs turbo-dom |
|---|---|---|---|---|---|---|
| payroll-app | 10,471 | ~252s | ~88s | ~20s | ~12× | ~4.4× |
| ui-design-components | 7,062 | ~173s | ~142s | ~36s | ~4.8× | ~3.9× |

All configurations passed 100% (10,471/0 and 7,062/0). On payroll-app, jsdom's `environment`
setup was 718s cumulative across workers vs turbo-dom's 33s.

## Isolate reuse (README, 2026-06-14, after v0.2.14)

An earlier A-B-B-A measurement on a quieter box put `isolate: false` (one isolate per worker)
on ui-design-components at ~1.5–1.6× over fresh isolation, against the same jsdom baseline.

## Jest compatibility (v0.2.11, 2026-06-13)

A NestJS + ts-jest backend (~7,150 tests): ~60s under turbo-test vs ~117s under jest (~1.9×)
on a clean box, with ~4,500 / 7,147 tests green (~63%) at the time.

## Profile hotspots

The warm-suite hotspot map (measured 2026-06) is in
[scripts/perf/README.md](../scripts/perf/README.md#hotspot-map-warm-suite-measured-2026-06).
