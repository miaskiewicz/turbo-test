import { describe, it, expect } from 'vitest';
import { HERE_DIR, HERE_FILE, joinFromHere } from './dirname-loader';

// Regression: a TypeScript module that reads __dirname/__filename at the top level loads through
// the ESM graph path (load_graph), where neither is a wrapper param the way they are on the CJS
// path. The runner injects Node-CJS-equivalent module-local bindings (__filename = the file,
// __dirname = its directory) when referenced — so both this spec AND its imported module resolve
// them instead of throwing `ReferenceError: __dirname is not defined`.
describe('node __dirname / __filename in an ESM-graph .ts module', () => {
  it('the imported module captured __dirname / __filename at top level', () => {
    expect(typeof HERE_DIR).toBe('string');
    expect(HERE_DIR.endsWith('/compat')).toBe(true);
    expect(HERE_FILE.endsWith('/dirname-loader.ts')).toBe(true);
    expect(joinFromHere('x.json')).toBe(`${HERE_DIR}/x.json`);
  });

  it('the spec file itself sees __dirname / __filename', () => {
    expect(typeof __dirname).toBe('string');
    expect(__dirname.endsWith('/compat')).toBe(true);
    expect(__filename.endsWith('/node-dirname.test.ts')).toBe(true);
  });
});
