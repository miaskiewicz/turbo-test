// App-style module that reads __dirname/__filename at TOP LEVEL. These files load through the
// ESM graph path (load_graph → compile_module), where — unlike the CJS wrapper — __dirname and
// __filename are NOT in scope by default. NestJS seed-runners / config loaders authored for the
// CJS world do exactly this; without injection it throws `ReferenceError: __dirname is not
// defined` at module eval. Imported by node-dirname.test.ts to lock in the fix.
export const HERE_DIR = __dirname;
export const HERE_FILE = __filename;

export function joinFromHere(seg: string): string {
  // Plain string join — avoids depending on node:path so the fixture stays self-contained.
  return `${__dirname}/${seg}`;
}
