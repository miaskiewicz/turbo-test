import { test, expect } from 'vitest';

const a = require('./full.node');

// V8 strings are capped at String::kMaxLength (< 2^29): napi_create_string_utf8, the
// named-property key conversion, a function name and napi_throw_error's message / code must
// report napi_generic_failure instead of crashing. [string, set key, get key, no string,
// no value, function name, no function, throw message, throw code, exception pending]
test('strings / keys / names / errors over String::kMaxLength fail with napi_generic_failure', () => {
  expect(a.huge()).toEqual([9, 9, 9, true, true, 9, true, 9, 9, false]);
});
