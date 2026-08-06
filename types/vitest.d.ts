// Module shim so `import { describe, it, expect, vi } from 'vitest'` type-checks under turbo-test
// WITHOUT the real `vitest` package installed. turbo-test resolves the `vitest` specifier from its
// own runtime at run time.
//
// To use, path-map the bare `vitest` specifier to this file in your tsconfig:
//
//   {
//     "compilerOptions": {
//       "paths": {
//         "vitest": ["./node_modules/@miaskiewicz/turbo-test/types/vitest.d.ts"]
//       }
//     }
//   }

import type {
  SuiteAPI,
  TestAPI,
  HookFunction,
  ExpectStatic,
  ExpectTypeOfStatic,
  AssertType,
  ViAPI,
} from './turbo-test-api';

export * from './turbo-test-api';

export declare const describe: SuiteAPI;
export declare const it: TestAPI;
export declare const test: TestAPI;
export declare const expect: ExpectStatic;
// Type-level assertions — checked by tsc, erased at run time (see turbo-test-api.d.ts).
export declare const expectTypeOf: ExpectTypeOfStatic;
export declare const assertType: AssertType;
export declare const vi: ViAPI;
export declare const beforeEach: HookFunction;
export declare const afterEach: HookFunction;
export declare const beforeAll: HookFunction;
export declare const afterAll: HookFunction;
export declare const assert: any;
