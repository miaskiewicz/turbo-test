// Regression: BUG-equality-state-leak-across-shard-files.md
// toEqual / toHaveBeenCalledWith must treat an own property whose value is `undefined`
// as ABSENT (jest/vitest semantics). toStrictEqual must keep it significant. Arrays never strip.
describe('undefined own-property stripping', () => {
  it('toEqual: explicit-undefined own prop is absent', () => {
    expect({ a: 1, b: undefined }).toEqual({ a: 1 });
    expect({ a: 1 }).toEqual({ a: 1, b: undefined });
  });

  it('toEqual: nested undefined stripped', () => {
    expect({ a: { b: 1, c: undefined } }).toEqual({ a: { b: 1 } });
  });

  it('toHaveBeenCalledWith strips undefined (the reported failure)', () => {
    const onImportComplete = vi.fn();
    onImportComplete({
      recordCount: 7,
      mappingState: { isComplete: true, hasValidationErrors: false },
      importTemplateId: undefined, // present as own prop, value undefined
      config: { version: 1, name: '' },
      uniqueIdentifierColumnIds: [],
    });
    expect(onImportComplete).toHaveBeenCalledWith({
      recordCount: 7,
      mappingState: { isComplete: true, hasValidationErrors: false },
      config: { version: 1, name: '' },
      uniqueIdentifierColumnIds: [],
    });
  });

  it('toStrictEqual keeps undefined significant', () => {
    let threw = false;
    try { expect({ a: 1, b: undefined }).toStrictEqual({ a: 1 }); } catch { threw = true; }
    expect(threw).toBe(true);
  });

  it('arrays: undefined element is NOT stripped (length matters)', () => {
    expect([1, undefined]).not.toEqual([1]);
    expect([1, undefined]).toEqual([1, undefined]);
  });

  it('undefined value vs real value still unequal', () => {
    expect({ a: undefined }).not.toEqual({ a: 1 });
  });
});
