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
  // tuple rows → spread into the callback's positional args, preserving per-position types
  // (`each([['x', true]])(…, (label: string, flag: boolean) => …)`). `readonly [...T]` forces
  // each row to infer as a tuple instead of widening to an element-union array.
  <T extends any[]>(cases: ReadonlyArray<readonly [...T]>): (
    name: string,
    fn: (...args: T) => any,
    timeout?: number,
  ) => void;
  (strings: TemplateStringsArray, ...values: any[]): (
    name: string,
    fn: (arg: any) => any,
    timeout?: number,
  ) => void;
  // single-value rows — incl. an explicit type arg over a non-array union
  // (`it.each<InviteType>(['a', 'b'])(…, (t) => …)`). Kept last so tuple rows infer first.
  <T>(cases: ReadonlyArray<T>): (
    name: string,
    fn: (arg: T) => any,
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

// vitest's hooks accept `() => any` — a callback may return a teardown fn, a chainable like
// `vi.useFakeTimers()` (returns ViAPI), or a Promise. Widen to match so `beforeEach(() =>
// vi.useFakeTimers())` type-checks.
export type HookFunction = (
  fn: () => unknown | Promise<unknown>,
  timeout?: number,
) => void;

// ---- mocks / spies ----------------------------------------------------------------------------

export interface MockContext<TArgs extends any[] = any[], TReturn = any> {
  calls: TArgs[];
  results: Array<{ type: 'return' | 'throw'; value: any }>;
  instances: any[];
  lastCall?: TArgs;
}

// vitest-4 generic form: the type parameter is the mocked function's *signature*, not an args
// tuple. `vi.fn<(a: string) => number>()` and `Mock<(a: string) => number>` both work; the no-arg
// default keeps a bare `Mock` usable.
export interface Mock<T extends (...args: any[]) => any = (...args: any[]) => any> {
  (...args: Parameters<T>): ReturnType<T>;
  mock: MockContext<Parameters<T>, ReturnType<T>>;
  getMockName(): string;
  mockName(name: string): this;
  mockClear(): this;
  mockReset(): this;
  mockRestore(): void;
  mockImplementation(fn: T): this;
  mockImplementationOnce(fn: T): this;
  mockReturnThis(): this;
  mockReturnValue(value: ReturnType<T>): this;
  mockReturnValueOnce(value: ReturnType<T>): this;
  mockResolvedValue(value: Awaited<ReturnType<T>>): this;
  mockResolvedValueOnce(value: Awaited<ReturnType<T>>): this;
  mockRejectedValue(value: any): this;
  mockRejectedValueOnce(value: any): this;
  getMockImplementation(): T | undefined;
}

// A mocked function keeps its original call signature AND gains the mock methods.
export type MockedFunction<T extends (...args: any[]) => any> = Mock<T> & T;
// A mocked object: every method becomes a MockedFunction, other props pass through.
export type MockedObject<T> = {
  [K in keyof T]: T[K] extends (...args: any[]) => any ? MockedFunction<T[K]> : T[K];
} & T;
export type Mocked<T> = T extends (...args: any[]) => any ? MockedFunction<T> : MockedObject<T>;

// The `importOriginal` callback handed to a `vi.mock` async factory — returns the real module,
// typed via the explicit type arg: `await importOriginal<typeof Mod>()`.
export type ImportOriginal = <T extends Record<string, any> = Record<string, any>>() => Promise<T>;

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
  fail(message?: string): never;
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
  fn(): Mock;
  fn<T extends (...args: any[]) => any>(impl?: T): Mock<T>;
  spyOn(obj: any, method: any): Mock;
  // async factory: `importOriginal<T>()` returns the real module typed as T
  // (`vi.mock('x', async (importOriginal) => ({ ...await importOriginal<typeof X>() }))`).
  mock(path: string, factory: (importOriginal: ImportOriginal) => any): void;
  mock(path: string, factory?: (...args: any[]) => any): void;
  unmock(path: string): void;
  doMock(path: string, factory: (importOriginal: ImportOriginal) => any): void;
  doMock(path: string, factory?: (...args: any[]) => any): void;
  doUnmock(path: string): void;
  // vi.hoisted: run a factory above the module's imports; returns its value (used for shared mock refs).
  hoisted<T>(factory: () => T): T;
  // function → MockedFunction (exposes mockReturnValue/.mock/…); anything else → MockedObject.
  mocked<T extends (...args: any[]) => any>(item: T, deep?: boolean): MockedFunction<T>;
  mocked<T>(item: T, deep?: boolean): MockedObject<T>;
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
