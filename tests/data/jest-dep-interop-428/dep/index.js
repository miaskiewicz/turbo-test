// A CommonJS module shaped like es-toolkit's `compat/get.js` after a bundler run:
// `module.exports` is marked `__esModule` and the real export sits on `.default`.
'use strict';
Object.defineProperty(exports, '__esModule', { value: true });
function get(obj, key) {
  return obj == null ? undefined : obj[key];
}
exports.default = get;
exports.get = get;
