import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { createRequire } from 'node:module';
import { buildSync } from 'esbuild';
import { createSSRApp } from 'vue';
import { renderToString } from '@vue/server-renderer';
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

test('Vue template asset imports survive the compiler render factory', async () => {
  const assetFile = fileURLToPath(new URL('../tests/data/vue/Asset.vue', import.meta.url));
  const result = spawnSync(process.execPath, [script], {
    input: JSON.stringify({ filename: assetFile, source: readFileSync(assetFile, 'utf8') }), encoding: 'utf8', timeout: 10_000,
  });
  assert.equal(result.status, 0, result.stderr);
  const compiled = JSON.parse(result.stdout);
  const output = buildSync({ stdin: { contents: compiled.code, resolveDir: dirname(assetFile), loader: 'ts' },
    bundle: true, packages: 'external', format: 'cjs', loader: { '.png': 'dataurl' }, write: false }).outputFiles[0].text;
  // External Vue is resolved through the real project, while asset imports use esbuild's actual loader.
  const require = createRequire(import.meta.url);
  const module = { exports: {} };
  new Function('require', 'module', 'exports', output)(require, module, module.exports);
  const html = await renderToString(createSSRApp(module.exports.default));
  const expected = 'data:image/png;base64,' + readFileSync(new URL('../tests/data/vue/logo.png', import.meta.url)).toString('base64');
  assert.ok(html.includes('src="' + expected + '"'), html);
  assert.ok(html.includes(expected + ' 1x'), html);
  assert.ok(html.includes(expected + ' 2x'), html);
});
