// Shared test-API type surface for turbo-test.
//
// turbo-test executes suites with its own native runtime — it never loads the real `vitest` or
// `@jest/globals` package. These declarations let a consumer's `tsc --noEmit` type-check the
// `describe`/`it`/`expect`/`vi`/`jest` surface WITHOUT keeping `vitest` (or `@types/jest`) in
// devDependencies just for types.
//
// Scope: a pragmatic, vitest/jest-compatible SUBSET. Matchers and mock signatures are typed
// permissively (arguments widen to `any`) so real consumer code type-checks and no FALSE errors
// are introduced — at the cost of some of the precise matcher-argument inference the upstream
// `vitest` types give you. If you need the full precise types, keep `vitest` as a types-only
// devDependency instead (see README → "TypeScript types").
//
// Consumed two ways:
//   - `types/vitest.d.ts`    → `declare module 'vitest'` / `'@jest/globals'` (for `import` sites)
//   - `types/globals.d.ts`   → ambient globals (replaces `types: ["vitest/globals"]`)

export type TestFunction = () => void | Promise<void>;

export interface TestOptions {
  timeout?: number;
  retry?: number;
  repeats?: number;
}

export interface TestEachFunction {
  <T extends any[] | [any]>(cases: ReadonlyArray<T>): (
    name: string,
    fn: (...args: T) => void | Promise<void>,
    timeout?: number,
  ) => void;
  (cases: ReadonlyArray<any>): (
    name: string,
    fn: (...args: any[]) => void | Promise<void>,
    timeout?: number,
  ) => void;
  (strings: TemplateStringsArray, ...values: any[]): (
    name: string,
    fn: (arg: any) => void | Promise<void>,
    timeout?: number,
  ) => void;
}

export interface TestAPI {
  (name: string, fn?: TestFunction, timeout?: number | TestOptions): void;
  only: TestAPI;
  skip: TestAPI;
  todo: (name: string, fn?: TestFunction) => void;
  concurrent: TestAPI;
  fails: TestAPI;
  each: TestEachFunction;
}

export interface SuiteAPI {
  (name: string, fn: () => void): void;
  only: SuiteAPI;
  skip: SuiteAPI;
  todo: (name: string) => void;
  concurrent: SuiteAPI;
  each: TestEachFunction;
}

export type HookFunction = (
  fn: () => void | Promise<void> | (() => void | Promise<void>),
  timeout?: number,
) => void;

// ---- mocks / spies ----------------------------------------------------------------------------

export interface MockContext<TArgs extends any[] = any[], TReturn = any> {
  calls: TArgs[];
  results: Array<{ type: 'return' | 'throw'; value: any }>;
  instances: any[];
  lastCall?: TArgs;
}

export interface Mock<TArgs extends any[] = any[], TReturn = any> {
  (...args: TArgs): TReturn;
  mock: MockContext<TArgs, TReturn>;
  getMockName(): string;
  mockName(name: string): this;
  mockClear(): this;
  mockReset(): this;
  mockRestore(): void;
  mockImplementation(fn: (...args: TArgs) => TReturn): this;
  mockImplementationOnce(fn: (...args: TArgs) => TReturn): this;
  mockReturnThis(): this;
  mockReturnValue(value: TReturn): this;
  mockReturnValueOnce(value: TReturn): this;
  mockResolvedValue(value: Awaited<TReturn>): this;
  mockResolvedValueOnce(value: Awaited<TReturn>): this;
  mockRejectedValue(value: any): this;
  mockRejectedValueOnce(value: any): this;
  getMockImplementation(): ((...args: TArgs) => TReturn) | undefined;
}

export type Mocked<T> = T;
export type MockedFunction<T extends (...args: any[]) => any> = Mock<Parameters<T>, ReturnType<T>> & T;

// ---- expect -----------------------------------------------------------------------------------

export interface Assertion {
  // matchers accept `any` so consumer code type-checks without precise upstream inference
  [matcher: string]: any;
  not: Assertion;
  resolves: Assertion;
  rejects: Assertion;
}

export interface ExpectStatic {
  (actual: any, message?: string): Assertion;
  extend(matchers: Record<string, (...args: any[]) => any>): void;
  assertions(count: number): void;
  hasAssertions(): void;
  soft(actual: any, message?: string): Assertion;
  any(constructor: any): any;
  anything(): any;
  objectContaining(expected: any): any;
  arrayContaining(expected: any[]): any;
  stringContaining(expected: string): any;
  stringMatching(expected: string | RegExp): any;
  closeTo(expected: number, precision?: number): any;
  not: {
    objectContaining(expected: any): any;
    arrayContaining(expected: any[]): any;
    stringContaining(expected: string): any;
    stringMatching(expected: string | RegExp): any;
  };
}

// ---- vi / jest controller --------------------------------------------------------------------

export interface ViAPI {
  fn<TArgs extends any[] = any[], TReturn = any>(
    impl?: (...args: TArgs) => TReturn,
  ): Mock<TArgs, TReturn>;
  spyOn(obj: any, method: any): Mock;
  mock(path: string, factory?: (...args: any[]) => any): void;
  unmock(path: string): void;
  doMock(path: string, factory?: (...args: any[]) => any): void;
  doUnmock(path: string): void;
  mocked<T>(item: T, deep?: boolean): Mocked<T>;
  importActual<T = any>(path: string): Promise<T>;
  importMock<T = any>(path: string): Promise<T>;
  clearAllMocks(): ViAPI;
  resetAllMocks(): ViAPI;
  restoreAllMocks(): ViAPI;
  isMockFunction(fn: any): boolean;
  resetModules(): ViAPI;
  // timers
  useFakeTimers(config?: any): ViAPI;
  useRealTimers(): ViAPI;
  isFakeTimers(): boolean;
  setSystemTime(time: number | Date): ViAPI;
  getMockedSystemTime(): Date | null;
  getRealSystemTime(): number;
  advanceTimersByTime(ms: number): ViAPI;
  advanceTimersByTimeAsync(ms: number): Promise<ViAPI>;
  advanceTimersToNextTimer(): ViAPI;
  advanceTimersToNextTimerAsync(): Promise<ViAPI>;
  runAllTimers(): ViAPI;
  runAllTimersAsync(): Promise<ViAPI>;
  runOnlyPendingTimers(): ViAPI;
  runOnlyPendingTimersAsync(): Promise<ViAPI>;
  clearAllTimers(): ViAPI;
  getTimerCount(): number;
  // env / globals
  stubEnv(name: string, value: string): ViAPI;
  unstubAllEnvs(): ViAPI;
  stubGlobal(name: string, value: any): ViAPI;
  unstubAllGlobals(): ViAPI;
  waitFor<T>(callback: () => T | Promise<T>, options?: any): Promise<T>;
  waitUntil<T>(callback: () => T | Promise<T>, options?: any): Promise<T>;
}

// jest's controller mirrors vi's surface (turbo-test backs both with the same implementation),
// plus jest-only sync helpers.
export interface JestAPI extends ViAPI {
  requireActual<T = any>(path: string): T;
  requireMock<T = any>(path: string): T;
  isolateModules(fn: () => void): void;
  isolateModulesAsync(fn: () => Promise<void>): Promise<void>;
  setTimeout(ms: number): JestAPI;
  retryTimes(n: number): JestAPI;
  replaceProperty(obj: any, key: string, value: any): { restore: () => void };
  now(): number;
}
