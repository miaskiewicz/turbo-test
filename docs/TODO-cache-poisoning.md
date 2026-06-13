# TODO: interrupted bundle writes poison the cache permanently

## Severity
High (correctness). A single interrupted run can wedge every later run with
phantom test failures until the cache is manually cleared.

## Symptom
After a turbo-test process is killed (Ctrl-C, OOM, CI timeout, `pkill`) mid-run,
subsequent runs fail with errors like:

```
/var/folders/.../turbo-test-cache/esb-<hash>.mjs:16201: Uncaught SyntaxError: Illegal return statement
```

Downstream, React components that import the corrupt bundle throw at render, so
whole files report 0 passed / N failed even though the source and the binary are
fine. Reproduced on 0.2.10 and 0.2.11 — not version-specific. Clears completely
the moment the cache dir is wiped.

## Root cause
The esbuild **bundle** cache files are written by esbuild's own `--outfile`,
NOT through `write_atomic` (temp + rename). If esbuild is killed mid-write, a
**truncated `.mjs` is left at the final cache path**. The cache-hit check
(`std::fs::read_to_string(&cache)`) then happily returns the partial file as a
"hit" forever, because the content hash key still matches the inputs.

Affected write sites in `src/runner.rs` (esbuild `--outfile` → cache path):
- `esbuild_bundle_full` (`esb-{hash}.mjs`)         ~line 593 / 620
- DOM boot bundle (`dom-boot-{hash}.mjs`)           ~line 717
- (verify) any other `--outfile` into `cache_dir()`

Note: the *transform* paths (`esbuild_transform_cjs`, `esbuild_bundle_dep_cjs`,
`read_transformed`) already go through `write_atomic` / capture stdout then
`write_atomic`, so they are safe. Only the `--outfile` bundles are exposed.

## Fix options (pick one, keep it generic)
1. **Atomic bundle outfile (preferred).** Point esbuild `--outfile` at a unique
   temp path in the cache dir, then `std::fs::rename` to the final cache name on
   success only. Rename is atomic on the same filesystem → a killed run leaves a
   stray temp file (harmless, ignored by the hit check), never a poisoned final.
   Mirrors `write_atomic` already used for the transform caches.
2. **Validate on read.** Cheap sanity check before trusting a cache hit (size > 0,
   maybe a trailing sentinel). Weaker — a truncated-but-parseable file slips through.
3. **Write to stdout instead of --outfile**, then `write_atomic` the captured
   bytes (same shape as the transform paths). Unifies all cache writes on one
   atomic path; costs a bit of memory for large bundles.

Recommendation: option 1 (or 3 for uniformity). Either makes interrupted runs
self-healing with no manual cache wipe.

## Test
- Start a run, kill it during bundling, re-run → must pass (currently fails).
- Add a unit/integration check: write a truncated file to a bundle cache path,
  confirm the next run regenerates rather than trusting it (only works if we add
  read-validation; the atomic-rename fix prevents the bad file existing at all).

## Harness note
`scripts/bench.sh` never kills runs for exactly this reason. Orphaned runs must be
left to drain naturally before wiping the cache.
