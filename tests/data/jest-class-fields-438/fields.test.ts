// ts-jest's TypeScript class field rule (tsconfig target: ES2022+ / useDefineForClassFields):
// an uninitialized field declaration becomes an own undefined property, visible to
// `hasOwnProperty`, `Object.keys`, spread and `%s`-style formatting (recovery.spec).
// turbo-test's decorator-metadata oxc path currently drops them (turbo-test#438).
function IsStr() { return (_t: any, _k: any) => {}; }
function IsOpt() { return (_t: any, _k: any) => {}; }

class ContractDto {
  @IsStr() contractType: string;
  @IsOpt() @IsStr() endDate?: string;
  @IsStr() withInit: string = 'x';
}

test('decorated TypeScript class keeps fields without initializer as own undefined properties', () => {
  const d = new ContractDto();
  expect(Object.prototype.hasOwnProperty.call(d, 'contractType')).toBe(true);
  expect(Object.prototype.hasOwnProperty.call(d, 'endDate')).toBe(true);
  expect(Object.keys(d).sort()).toEqual(['contractType', 'endDate', 'withInit']);
  expect({ ...d }).toEqual({ contractType: undefined, endDate: undefined, withInit: 'x' });
});
