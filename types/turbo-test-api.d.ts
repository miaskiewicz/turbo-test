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

// The context object vitest passes as the first arg to every test body and hook (`it('x', ({
// expect, task }) => …)`, `beforeEach((ctx) => …)`). Kept permissive (index signature) so any
// destructure type-checks; the common members are typed for editor help.
export interface TestContext {
  readonly task: any;
  readonly signal: AbortSignal;
  readonly expect: ExpectStatic;
  readonly skip: (note?: string) => void;
  readonly annotate: (...args: any[]) => any;
  readonly onTestFailed: (fn: (ctx: TestContext) => any, timeout?: number) => void;
  readonly onTestFinished: (fn: (ctx: TestContext) => any, timeout?: number) => void;
  [key: string]: any;
}

// vitest's `TestFunction` receives the test context (`(context) => Awaitable<any> | void`); a
// no-arg `() => …` body is still assignable (fewer params).
export type TestFunction = (context: TestContext) => any;

export interface TestOptions {
  timeout?: number;
  retry?: number;
  repeats?: number;
}

// `it.each` typing — lifted verbatim from vitest (`@vitest/runner`'s `ExtractEachCallbackArgs` /
// `EachFunctionReturn` / `TestEachFunction`) so behavior matches the real runner exactly: tuple
// rows (const or not, up to 10 cols) map to precise positional args; everything else (ragged /
// heterogeneous / >10 cols) lands on the permissive `fallback`/`T[]` overload with array args.
export type ExtractEachCallbackArgs<T extends ReadonlyArray<any>> = {
  1: [T[0]];
  2: [T[0], T[1]];
  3: [T[0], T[1], T[2]];
  4: [T[0], T[1], T[2], T[3]];
  5: [T[0], T[1], T[2], T[3], T[4]];
  6: [T[0], T[1], T[2], T[3], T[4], T[5]];
  7: [T[0], T[1], T[2], T[3], T[4], T[5], T[6]];
  8: [T[0], T[1], T[2], T[3], T[4], T[5], T[6], T[7]];
  9: [T[0], T[1], T[2], T[3], T[4], T[5], T[6], T[7], T[8]];
  10: [T[0], T[1], T[2], T[3], T[4], T[5], T[6], T[7], T[8], T[9]];
  fallback: Array<T extends ReadonlyArray<infer U> ? U : any>;
}[T extends Readonly<[any]> ? 1
  : T extends Readonly<[any, any]> ? 2
  : T extends Readonly<[any, any, any]> ? 3
  : T extends Readonly<[any, any, any, any]> ? 4
  : T extends Readonly<[any, any, any, any, any]> ? 5
  : T extends Readonly<[any, any, any, any, any, any]> ? 6
  : T extends Readonly<[any, any, any, any, any, any, any]> ? 7
  : T extends Readonly<[any, any, any, any, any, any, any, any]> ? 8
  : T extends Readonly<[any, any, any, any, any, any, any, any, any]> ? 9
  : T extends Readonly<[any, any, any, any, any, any, any, any, any, any]> ? 10
  : 'fallback'];

export interface EachFunctionReturn<T extends any[]> {
  (name: string | Function, fn: (...args: T) => any, options?: number): void;
  (name: string | Function, options: TestOptions, fn: (...args: T) => any): void;
}

export interface TestEachFunction {
  <T extends any[] | [any]>(cases: ReadonlyArray<T>): EachFunctionReturn<T>;
  <T extends ReadonlyArray<any>>(cases: ReadonlyArray<T>): EachFunctionReturn<ExtractEachCallbackArgs<T>>;
  <T>(cases: ReadonlyArray<T>): EachFunctionReturn<T[]>;
  (...args: [TemplateStringsArray, ...any]): EachFunctionReturn<any[]>;
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
  (name: string, fn: () => any): void;
  only: SuiteAPI;
  skip: SuiteAPI;
  todo: (name: string) => void;
  concurrent: SuiteAPI;
  each: TestEachFunction;
}

// vitest's hooks pass a context (`beforeEach((ctx, suite) => …)`) and accept any return — a
// teardown fn, a chainable like `vi.useFakeTimers()` (returns ViAPI), or a Promise. The callback
// args are optional, so `beforeEach(() => vi.useFakeTimers())` and `afterEach(async () => …)`
// both type-check.
export type HookFunction = (
  fn: (context: TestContext, ...rest: any[]) => any,
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
  // forward-compat: vitest exposes more statics (poll, unreachable, assertType,
  // addSnapshotSerializer, getState/setState, …). Permit any `expect.X` rather than error.
  [key: string]: any;
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

// ---- expectTypeOf / assertType ----------------------------------------------------------------
//
// Type-level assertions. They are erased at compile time and have NO runtime behavior (turbo-test
// backs `expectTypeOf` with a chainable no-op, same observable result as vitest) — all the checking
// happens here, in the types. A failed assertion surfaces as an arity error on the call
// ("Expected 1 arguments, but got 0") because the mismatch branch demands an extra argument that
// cannot be produced.

/** Strict type equality — distinguishes `any`/`unknown`/`never` and optional-vs-undefined. */
export type TypeEqual<X, Y> = (<T>() => T extends X ? 1 : 2) extends <T>() => T extends Y ? 1 : 2
  ? true
  : false;
export type TypeExtends<A, B> = [A] extends [B] ? true : false;
export type IsAny<T> = 0 extends 1 & T ? true : false;
export type IsNullable<T> = null extends T ? true : undefined extends T ? true : false;
type Negate<B extends boolean> = B extends true ? false : true;
/** The failure payload: an argument the caller has no way to construct. */
type TypeMismatch = { readonly __turboTestTypeAssertionFailed: unique symbol };
/** `[]` when the check holds (call takes no extra arg), `[TypeMismatch]` when it doesn't. */
type TypeArgs<Got extends boolean, Want extends boolean> = Got extends Want ? [] : [TypeMismatch];

export interface ExpectTypeOf<Actual, Positive extends boolean> {
  /** Inverts every assertion below it: `expectTypeOf<A>().not.toEqualTypeOf<B>()`. */
  not: ExpectTypeOf<Actual, Negate<Positive>>;

  toEqualTypeOf<Expected>(
    expected: Expected,
    ...args: TypeArgs<TypeEqual<Actual, Expected>, Positive>
  ): void;
  toEqualTypeOf<Expected>(...args: TypeArgs<TypeEqual<Actual, Expected>, Positive>): void;

  toMatchTypeOf<Expected>(
    expected: Expected,
    ...args: TypeArgs<TypeExtends<Actual, Expected>, Positive>
  ): void;
  toMatchTypeOf<Expected>(...args: TypeArgs<TypeExtends<Actual, Expected>, Positive>): void;

  /** vitest 3+ spelling of `toMatchTypeOf`. */
  toExtend<Expected>(expected: Expected, ...args: TypeArgs<TypeExtends<Actual, Expected>, Positive>): void;
  toExtend<Expected>(...args: TypeArgs<TypeExtends<Actual, Expected>, Positive>): void;
  toMatchObjectType<Expected>(...args: TypeArgs<TypeExtends<Actual, Expected>, Positive>): void;

  /** Asserts the key exists, and narrows the chain to that property's type. */
  toHaveProperty<K extends PropertyKey>(
    key: K,
    ...args: TypeArgs<K extends keyof Actual ? true : false, Positive>
  ): ExpectTypeOf<K extends keyof Actual ? Actual[K] : never, true>;

  toBeString(...args: TypeArgs<TypeExtends<Actual, string>, Positive>): void;
  toBeNumber(...args: TypeArgs<TypeExtends<Actual, number>, Positive>): void;
  toBeBoolean(...args: TypeArgs<TypeExtends<Actual, boolean>, Positive>): void;
  toBeBigInt(...args: TypeArgs<TypeExtends<Actual, bigint>, Positive>): void;
  toBeSymbol(...args: TypeArgs<TypeExtends<Actual, symbol>, Positive>): void;
  toBeVoid(...args: TypeArgs<TypeExtends<Actual, void>, Positive>): void;
  toBeNull(...args: TypeArgs<TypeEqual<Actual, null>, Positive>): void;
  toBeUndefined(...args: TypeArgs<TypeEqual<Actual, undefined>, Positive>): void;
  toBeNullable(...args: TypeArgs<IsNullable<Actual>, Positive>): void;
  toBeAny(...args: TypeArgs<IsAny<Actual>, Positive>): void;
  toBeUnknown(...args: TypeArgs<TypeEqual<Actual, unknown>, Positive>): void;
  toBeNever(...args: TypeArgs<TypeEqual<Actual, never>, Positive>): void;
  toBeObject(...args: TypeArgs<TypeExtends<Actual, object>, Positive>): void;
  toBeArray(...args: TypeArgs<TypeExtends<Actual, readonly any[]>, Positive>): void;
  toBeFunction(...args: TypeArgs<TypeExtends<Actual, (...a: any[]) => any>, Positive>): void;

  /** Callable/constructible with exactly these arguments (arity + types checked by tsc). */
  toBeCallableWith: Actual extends (...a: infer P) => any ? (...args: P) => void : never;
  toBeConstructibleWith: Actual extends new (...a: infer P) => any ? (...args: P) => void : never;

  // navigation — each narrows the chain to a sub-type of `Actual`
  items: ExpectTypeOf<Actual extends readonly (infer E)[] ? E : never, Positive>;
  returns: ExpectTypeOf<Actual extends (...a: any[]) => infer R ? R : never, Positive>;
  parameters: ExpectTypeOf<Actual extends (...a: infer P) => any ? P : never, Positive>;
  resolves: ExpectTypeOf<Actual extends PromiseLike<infer R> ? R : never, Positive>;
  instance: ExpectTypeOf<Actual extends new (...a: any[]) => infer I ? I : never, Positive>;
  constructorParameters: ExpectTypeOf<Actual extends new (...a: infer P) => any ? P : never, Positive>;
  thisParameter: ExpectTypeOf<Actual extends (this: infer T, ...a: any[]) => any ? T : never, Positive>;
  guards: ExpectTypeOf<Actual extends (v: any, ...a: any[]) => v is infer G ? G : never, Positive>;
  asserts: ExpectTypeOf<Actual, Positive>;
  branded: ExpectTypeOf<Actual, Positive>;
}

export interface ExpectTypeOfStatic {
  /** Value form: `expectTypeOf(user).toEqualTypeOf<User>()`. */
  <Actual>(actual: Actual): ExpectTypeOf<Actual, true>;
  /** Type-arg form: `expectTypeOf<User>().toHaveProperty('id')`. */
  <Actual>(): ExpectTypeOf<Actual, true>;
}

/** Asserts `value` is assignable to `T`; a compile-time no-op otherwise. */
export type AssertType = <T>(value: T) => void;

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
