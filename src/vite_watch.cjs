// Collect the module edges Vite plugins register with addWatchFile. No tests are executed.
const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');

function canVisit(id) {
  if (id.startsWith('\0')) return true;
  try { return fs.statSync(id).isFile(); } catch { return false; }
}

async function visitChild(env, child, visited, files, parent) {
  const moduleId = child.id;
  const id = moduleId?.split('?')[0];
  if (!id) throw new Error(`Vite returned an unresolved dependency for ${parent}`);
  if (path.isAbsolute(id)) files.add(id);
  if (canVisit(id)) await visit(env, moduleId, visited, files, child.url);
}

async function visit(env, file, visited, files, requestedUrl) {
  if (visited.has(file)) return;
  visited.add(file);
  const fallbackUrl = file.startsWith('\0') ? `/@id/__x00__${file.slice(1)}` : `/@fs/${file}`;
  await env.transformRequest(requestedUrl || fallbackUrl);
  const node = env.moduleGraph.getModuleById(file);
  if (!node) throw new Error(`Vite did not register ${file}`);
  for (const child of node.importedModules) await visitChild(env, child, visited, files, file);
}

function addGlobalWatches(server, env, result) {
  const attributed = new Set(Object.values(result).flat());
  const environments = Object.values(server.environments || { default: env });
  const watches = environments.flatMap(current => {
    if (!current.pluginContainer?.watchFiles) throw new Error('Vite does not expose all plugin watch files');
    const files = [...current.pluginContainer.watchFiles];
    return current === env ? files.filter(file => !attributed.has(file)) : files;
  });
  for (const files of Object.values(result)) files.push(...watches);
}

async function collect(server, env, tests) {
  if (!env.pluginContainer?.watchFiles) throw new Error('Vite does not expose plugin watch files');
  const result = {};
  for (const test of tests) {
    const files = new Set();
    const visited = new Set();
    for (const entry of test.entries) await visit(env, entry, visited, files);
    result[test.file] = [...files];
  }
  addGlobalWatches(server, env, result);
  return result;
}

async function main() {
  const input = JSON.parse(fs.readFileSync(0, 'utf8'));
  process.env.NODE_ENV ||= 'test';
  const requireFromProject = createRequire(path.join(input.root, 'package.json'));
  const vite = await import(pathToFileURL(requireFromProject.resolve('vite')).href);
  const server = await vite.createServer({ root: input.root, configFile: input.config, mode: 'test', environments: { __vitest__: { dev: {} } }, logLevel: 'silent', server: { middlewareMode: true, watch: null }, optimizeDeps: { noDiscovery: true } });
  try {
    const env = server.environments?.__vitest__ || server.environments?.ssr || { transformRequest: server.transformRequest.bind(server), moduleGraph: server.moduleGraph, pluginContainer: server.pluginContainer };
    const result = await collect(server, env, input.tests);
    process.stdout.write(`\n__TURBO_VITE_WATCH__${JSON.stringify(result)}\n`);
  } finally { await server.close(); }
}
main().catch(error => { console.error(error.stack || String(error)); process.exitCode = 1; });
