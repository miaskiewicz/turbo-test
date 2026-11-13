import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../scripts/vue-transform.cjs', import.meta.url));
const filename = fileURLToPath(new URL('../tests/data/vue/Counter.vue', import.meta.url));
function failure(source) {
  const result = spawnSync(process.execPath, [script], {
    input: JSON.stringify({ filename, source }), encoding: 'utf8', timeout: 10_000,
  });
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  return result.stderr;
}

test('Vue compiler reports real TypeScript script syntax failures', () => {
  assert.match(failure('<script setup lang="ts">const invalid: = 1;</script><template><p>broken</p></template>'), /Unexpected token/);
});
test('Vue compiler reports malformed template structure', () => {
  assert.match(failure('<template><div><span></div></template>'), /missing end tag/);
});
test('Vue loader rejects unsupported style preprocessing explicitly', () => {
  assert.match(failure('<template><p>styled</p></template><style scoped lang="scss">$c:red;p{color:$c}</style>'), /Unsupported Vue style language: scss/);
});
test('Vue loader rejects unsupported external blocks explicitly', () => {
  assert.match(failure('<script src="./external.js"></script><template><p>external</p></template>'), /External Vue SFC blocks are not supported/);
});
