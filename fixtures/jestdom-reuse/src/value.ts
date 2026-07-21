// A trivial instrumentable source module so --coverage has something to instrument
// (a fixture with 0 instrumented files hard-fails by design).
export function theValue(): string {
  return 'custom';
}
