// Deliberate SyntaxError: imported by imports-broken.test.ts to prove a module that fails to
// compile is a reported load error, not a V8 abort.
export const x = ;
