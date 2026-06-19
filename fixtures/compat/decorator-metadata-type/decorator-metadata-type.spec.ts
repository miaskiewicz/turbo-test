import { describe, it, expect } from '@jest/globals';
import { Model } from './model';
describe('decorator metadata referencing an imported interface type', () => {
  it('loads despite @Col() design:type on an interface-typed field', () => {
    expect(new Model()).toBeInstanceOf(Model);
  });
});
