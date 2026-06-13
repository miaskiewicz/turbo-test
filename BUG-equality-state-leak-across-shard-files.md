# Bug: `toEqual` / `toHaveBeenCalledWith` undefined-key stripping leaks across files in a sharded worker

**Status:** open
**Reported from:** flux-payroll/payroll-app PR #187 (CI job "Unit tests (shard 3/4)")
**turbo-test:** `@miaskiewicz/turbo-test@^0.2.10`
**Severity:** medium (false-negative test failures; order/shard-dependent, not reproducible in isolation)

---

## TL;DR

When several test files run in the same sharded worker, the equality semantics used
by `toEqual` and `toHaveBeenCalledWith` change partway through the run. Specifically
the standard **"an own property whose value is `undefined` is treated as absent"**
rule (i.e. `{ a: 1, b: undefined }` deep-equals `{ a: 1 }`) stops being applied.

A later test that asserts on an object containing an explicit `undefined` property
then fails its deep-equality even though the values are, by jest/vitest semantics,
equal. The printed `expected` and `received` are **byte-identical** (because the
`undefined` property isn't rendered), which makes the failure look impossible.

The state is **not reset between files** in the worker, so the failure only appears
when a "polluting" file runs earlier in the same shard. It never reproduces when the
affected file is run alone.

## Symptom (what we saw)

CI shard 3/4 failed one test; the same test passed locally in isolation and passed
on shards 1/2/4. The assertion output:

```
✗ useDataImportModal completion > serializes the manual mapping into a config and reports it then closes:
  expected spy to have been called with
    [{"recordCount":7,"mappingState":{"isComplete":true,"hasValidationErrors":false},
      "file":{"_parts":["x"],"type":"","size":1,"name":"roster.csv","lastModified":0},
      "config":{"version":1,"name":""},"uniqueIdentifierColumnIds":[]}];
  received 1 call(s):
    [[{"recordCount":7,"mappingState":{"isComplete":true,"hasValidationErrors":false},
      "file":{"_parts":["x"],"type":"","size":1,"name":"roster.csv","lastModified":0},
      "config":{"version":1,"name":""},"uniqueIdentifierColumnIds":[]}]]
```

`expected` and `received` print identically → looks like a false failure.

## Root cause (confirmed by elimination)

The asserted-against object is actually:

```js
// produced by the code under test
{
  recordCount: 7,
  mappingState: { isComplete: true, hasValidationErrors: false },
  file,
  importTemplateId: undefined,   // <-- present as an OWN property, value undefined
  config: { version: 1, name: '' },
  uniqueIdentifierColumnIds: [],
}
```

The test asserts against the same object **without** `importTemplateId`:

```js
expect(onImportComplete).toHaveBeenCalledWith({
  recordCount: 7,
  mappingState: { isComplete: true, hasValidationErrors: false },
  file,
  config: { version: 1, name: '' },
  uniqueIdentifierColumnIds: [],
});
```

Under jest/vitest semantics `toEqual`/`toHaveBeenCalledWith` strip `undefined`-valued
own properties, so `{importTemplateId: undefined, ...rest}` equals `{...rest}` → pass.

We bisected which value broke the whole-object compare by asserting each field on its
own against the captured `mock.calls[0][0]` (all run in the SAME shard-3 context):

| assertion                                              | shard 3 result |
| ------------------------------------------------------ | -------------- |
| `expect(reported.recordCount).toBe(7)`                 | ✅ pass        |
| `expect(reported.mappingState).toEqual({...})`         | ✅ pass        |
| `expect(reported.file).toBe(file)`                     | ✅ pass        |
| `expect(reported.file).toEqual(file)` (deep)           | ✅ pass        |
| `expect(reported.config).toEqual({version:1,name:''})` | ✅ pass        |
| `expect(reported.uniqueIdentifierColumnIds).toEqual([])`| ✅ pass        |
| `expect(reported).toEqual({...5 keys, no importTemplateId})` | ❌ **fail** |
| `expect(onImportComplete).toHaveBeenCalledWith({...5 keys})`  | ❌ **fail** |

Every constituent value compares equal, including the `File` instance compared deeply.
Only the **whole-object** compare fails — because that is the only comparison that has
to reconcile the extra `importTemplateId: undefined` own-property. The conclusion: in
shard 3 the equality function is **no longer stripping `undefined` own-properties**, so
the received object has 6 keys and the expected has 5 → not equal.

In isolation the same whole-object compare passes, so the `undefined`-stripping rule is
on by default and is being **turned off by some earlier test file in the shard** and not
restored for subsequent files.

## Why it is a turbo-test isolation bug

- Default behavior (clean worker): `undefined` own-props stripped → whole-object equal → pass.
- After an earlier shard file runs: stripping disabled → whole-object unequal → fail.
- The change persists across the file boundary → state is shared, not reset per file.

Likely culprits to check inside turbo-test's `expect` integration (threads pool):

1. The equality config / options object backing `equals()` (the `iterableEquality`,
   `subsetEquality`, and the `undefined`-property handling) is a module-level singleton
   that is mutated by a matcher and never reset between files.
2. `expect.addEqualityTesters(...)` / `expect.extend(...)` / custom asymmetric matchers
   registered by one test file are not torn down before the next file in the same worker.
3. The "strip undefined" pass (jest's `equals(a, b, [iterableEquality], true)` — the
   trailing `strictCheck`/`undefined`-handling argument) is being toggled globally
   somewhere and leaking.

Vitest avoids this by isolating module + `expect` state per file. Under turbo-test's
`pool: 'threads'` (this project's config) the worker is reused across files, so any
global `expect`/equality state must be snapshotted and restored at each file boundary.

## Minimal repro shape

```js
// File A (runs first in the shard): does SOMETHING that mutates global expect
//   equality state (e.g. expect.extend / addEqualityTesters / a matcher that flips
//   the undefined-stripping option) and does not restore it.

// File B (runs later in the same shard):
it('treats an explicit-undefined own property as absent', () => {
  const received = { a: 1, b: undefined };
  expect(received).toEqual({ a: 1 });   // passes alone, FAILS after File A in same worker
});
```

Repro command used in payroll-app:

```
CI=1 npx turbo-test --shard 3/4
# → 1 failed: useDataImportModal.completion.test.ts
#   (passes as:  npx turbo-test src/.../useDataImportModal.completion.test.ts)
```

## Suggested fixes (turbo-test side)

- Reset the `expect` equality registry (custom testers, `expect.extend` matchers,
  asymmetric-matcher state) at the start of every test file in a reused worker.
- Ensure the `undefined`-own-property-stripping rule in the `equals` path is derived
  from per-invocation options, not a shared mutable singleton.
- Add a worker-reuse isolation test: run two files in one worker where the first
  registers an equality tester / extends expect, and assert the second file sees the
  default semantics.

## Workaround (consumer side, not applied here)

Asserting field-by-field against `mock.calls[0][0]` (comparing the `File` by reference
with `toBe`, plain objects with `toEqual`) sidesteps the whole-object compare and is
immune to the leak. Per the repo owner's request we have **left the natural
`toHaveBeenCalledWith({...})` assertion in place (currently failing in shard 3)** so the
turbo-test fix can be validated against it.

Affected test: `src/components/modals/DataImportModal/useDataImportModal.completion.test.ts`
→ "serializes the manual mapping into a config and reports it then closes".
