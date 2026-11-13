'use strict';
// Real project Vue compiler, isolated from the runner's own package dependencies.
const fs = require('node:fs');
const { createRequire } = require('node:module');
const { createHash } = require('node:crypto');

function compilerFor(filename) {
  const project = createRequire(filename);
  let entry;
  try { entry = project.resolve('vue/compiler-sfc'); }
  catch { entry = project.resolve('@vue/compiler-sfc'); }
  const compiler = project(entry);
  if (!compiler.version.startsWith('3.')) throw new Error('Vue 3 is required');
  return compiler;
}
function check(errors) {
  if (errors.length) throw new Error(errors.map((error) => error.message || String(error)).join('\n'));
}
function blockSupported(block) {
  if (!block) return;
  if (block.src) throw new Error('External Vue SFC blocks are not supported');
  if (block.lang && !['js', 'ts'].includes(block.lang)) throw new Error('Unsupported Vue script language: ' + block.lang);
}
function scriptPart(compiler, descriptor, id) {
  blockSupported(descriptor.script);
  blockSupported(descriptor.scriptSetup);
  if (!descriptor.script && !descriptor.scriptSetup) return { code: 'const __sfc__ = {};', bindings: {} };
  const script = compiler.compileScript(descriptor, { id, inlineTemplate: false, isProd: false });
  return { code: compiler.rewriteDefault(script.content, '__sfc__', ['typescript']), bindings: script.bindings };
}
function templatePart(compiler, descriptor, filename, id, bindings) {
  if (!descriptor.template) return '';
  if (descriptor.template.src || descriptor.template.lang) throw new Error('External or preprocessed Vue templates are not supported');
  const template = compiler.compileTemplate({
    source: descriptor.template.content, filename, id,
    scoped: descriptor.styles.some((style) => style.scoped), isProd: false,
    compilerOptions: { mode: 'function', prefixIdentifiers: true, scopeId: null, hoistStatic: false,
      runtimeGlobalName: '__ttVue', bindingMetadata: bindings },
  });
  check(template.errors);
  return "import * as __ttVue from 'vue';\n__sfc__.render = (() => {\n" + template.code + '\n})();';
}
function stylePart(compiler, style, filename, id) {
  if (style.src || style.module) throw new Error('External or module Vue styles are not supported');
  if (style.lang && style.lang !== 'css') throw new Error('Unsupported Vue style language: ' + style.lang);
  const result = compiler.compileStyle({ source: style.content, filename, id, scoped: Boolean(style.scoped), isProd: false });
  check(result.errors);
  return result.code;
}
function injectStyle(css, id) {
  return `if (typeof document !== 'undefined' && !document.querySelector('style[data-turbo-vue="${id}"]')) {
    const style = document.createElement('style');
    style.setAttribute('data-turbo-vue', '${id}');
    style.textContent = ${JSON.stringify(css)};
    document.head.appendChild(style);
  }`;
}
function compile(input) {
  const compiler = compilerFor(input.filename);
  const parsed = compiler.parse(input.source, { filename: input.filename });
  check(parsed.errors);
  const descriptor = parsed.descriptor;
  const id = 'data-v-' + createHash('sha256').update(input.filename + '\0' + input.source).digest('hex').slice(0, 10);
  const script = scriptPart(compiler, descriptor, id);
  const template = templatePart(compiler, descriptor, input.filename, id, script.bindings);
  const css = descriptor.styles.map((style) => stylePart(compiler, style, input.filename, id)).join('\n');
  const scope = descriptor.styles.some((style) => style.scoped) ? '__sfc__.__scopeId = ' + JSON.stringify(id) + ';' : '';
  const code = [script.code, template, scope, '__sfc__.__file = ' + JSON.stringify(input.filename) + ';',
    css ? injectStyle(css, id) : '', 'export default __sfc__;'].join('\n');
  return { code: '// Vue compiler ' + compiler.version + '\n' + code, version: compiler.version };
}
try { process.stdout.write(JSON.stringify(compile(JSON.parse(fs.readFileSync(0, 'utf8'))))); }
catch (error) { process.stderr.write(error.message + '\n'); process.exitCode = 1; }
