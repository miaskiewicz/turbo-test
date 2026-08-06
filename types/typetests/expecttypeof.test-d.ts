// Compile-only guard for the `expectTypeOf` / `assertType` type shim (issue #16). NOT run by the
// test runner — checked by `test/compat-types.test.mjs` via `tsc --noEmit --strict`.
//
// Everything here must type-check. The NEGATIVE cases (assertions that SHOULD fail) are marked
// with `@ts-expect-error`, so the guard fails in both directions: a shim that stops catching real
// mismatches leaves an unused `@ts-expect-error` and tsc errors on it.
/// <reference path="../globals.d.ts" />

type User = { id: string; name: string };

// ---- the forms from the issue -----------------------------------------------------------------

expectTypeOf<User>().toHaveProperty('id');
expectTypeOf<User>().toEqualTypeOf<{ id: string; name: string }>();
expectTypeOf<number>().toEqualTypeOf<number>();

// value form (infers Actual from the argument)
expectTypeOf({ id: 'a', name: 'b' }).toEqualTypeOf<User>();
expectTypeOf(1).toEqualTypeOf(2);

// ---- .not -------------------------------------------------------------------------------------

expectTypeOf<User>().not.toEqualTypeOf<{ id: string }>();
expectTypeOf<User>().not.toHaveProperty('missing');
expectTypeOf<string>().not.toBeNumber();

// ---- mismatches are real compile errors -------------------------------------------------------

// @ts-expect-error — string is not number
expectTypeOf<string>().toEqualTypeOf<number>();
// @ts-expect-error — `age` is not a key of User
expectTypeOf<User>().toHaveProperty('age');
// @ts-expect-error — the types DO match, so `.not` must fail
expectTypeOf<number>().not.toEqualTypeOf<number>();
// @ts-expect-error — a string is not a number
expectTypeOf<string>().toBeNumber();

// ---- primitive / special-type assertions ------------------------------------------------------

expectTypeOf<string>().toBeString();
expectTypeOf<number>().toBeNumber();
expectTypeOf<boolean>().toBeBoolean();
expectTypeOf<null>().toBeNull();
expectTypeOf<undefined>().toBeUndefined();
expectTypeOf<string | null>().toBeNullable();
expectTypeOf<any>().toBeAny();
expectTypeOf<unknown>().toBeUnknown();
expectTypeOf<never>().toBeNever();
expectTypeOf<User>().toBeObject();
expectTypeOf<string[]>().toBeArray();
expectTypeOf<() => void>().toBeFunction();
expectTypeOf<void>().toBeVoid();

// ---- assignability (toMatchTypeOf / toExtend) -------------------------------------------------

expectTypeOf<User>().toMatchTypeOf<{ id: string }>();
expectTypeOf<User>().toExtend<{ id: string }>();
// @ts-expect-error — User has no `age`, so it does not extend this
expectTypeOf<User>().toExtend<{ age: number }>();

// ---- navigation: the chain narrows -------------------------------------------------------------

expectTypeOf<User[]>().items.toEqualTypeOf<User>();
expectTypeOf<() => User>().returns.toEqualTypeOf<User>();
expectTypeOf<(a: string, b: number) => void>().parameters.toEqualTypeOf<[string, number]>();
expectTypeOf<Promise<User>>().resolves.toEqualTypeOf<User>();
expectTypeOf<User>().toHaveProperty('id').toBeString();

class Widget {
  constructor(public size: number) {}
}
expectTypeOf<typeof Widget>().instance.toEqualTypeOf<Widget>();
expectTypeOf<typeof Widget>().constructorParameters.toEqualTypeOf<[number]>();

// ---- callable/constructible arity ---------------------------------------------------------------

expectTypeOf<(a: string) => void>().toBeCallableWith('ok');
// @ts-expect-error — wrong argument type
expectTypeOf<(a: string) => void>().toBeCallableWith(1);
expectTypeOf<typeof Widget>().toBeConstructibleWith(3);

// ---- assertType ---------------------------------------------------------------------------------

assertType<User>({ id: 'a', name: 'b' });
// @ts-expect-error — missing `name`
assertType<User>({ id: 'a' });
