// Module shim so `import { jest, describe, it, expect } from '@jest/globals'` type-checks under
// turbo-test WITHOUT the real `@jest/globals` package installed. turbo-test resolves the
// `@jest/globals` specifier from its own runtime at run time (the same builtin that backs `vitest`).
//
// To use, path-map the bare `@jest/globals` specifier to this file in your tsconfig:
//
//   {
//     "compilerOptions": {
//       "paths": {
//         "@jest/globals": ["./node_modules/@miaskiewicz/turbo-test/types/jest-globals.d.ts"]
//       }
//     }
//   }

import type {
  SuiteAPI,
  TestAPI,
  HookFunction,
  ExpectStatic,
  JestAPI,
} from './turbo-test-api';

export * from './turbo-test-api';

export declare const describe: SuiteAPI;
export declare const it: TestAPI;
export declare const test: TestAPI;
export declare const expect: ExpectStatic;
export declare const jest: JestAPI;
export declare const beforeEach: HookFunction;
export declare const afterEach: HookFunction;
export declare const beforeAll: HookFunction;
export declare const afterAll: HookFunction;
