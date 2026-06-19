// Ambient test globals — the drop-in replacement for `types: ["vitest/globals"]`.
//
// turbo-test injects describe/it/expect/vi/jest (and the hook functions) as globals at run time,
// the same as vitest/jest with globals enabled. Reference this instead of `vitest/globals` so the
// globals type-check without the `vitest` package installed:
//
//   {
//     "compilerOptions": {
//       "types": ["@miaskiewicz/turbo-test/globals"]
//     }
//   }

import type {
  SuiteAPI,
  TestAPI,
  HookFunction,
  ExpectStatic,
  ViAPI,
  JestAPI,
} from './turbo-test-api';

export {};

declare global {
  const describe: SuiteAPI;
  const it: TestAPI;
  const test: TestAPI;
  const expect: ExpectStatic;
  const vi: ViAPI;
  // turbo-test also injects the jest controller as a global (jest-project parity).
  const jest: JestAPI;
  const beforeEach: HookFunction;
  const afterEach: HookFunction;
  const beforeAll: HookFunction;
  const afterAll: HookFunction;
}
