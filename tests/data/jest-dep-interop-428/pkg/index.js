// A Node ES-module package (`"type":"module"`) importing a CommonJS dependency whose
// `module.exports` is marked `__esModule` (as a bundler / tsc output is), with its real export
// on `.default`.  Under jest/ts-jest's `_interopRequireDefault`, `.default` is the dep's own
// `.default` (the `get` function).  Node's native ESM->CJS interop differs — `.default` is the
// whole `module.exports`.  turbo-test must follow jest's rule when a jest project transforms
// such a package (turbo-test#428).
import get from 'dep';
export const callGet = (obj, key) => get(obj, key);
