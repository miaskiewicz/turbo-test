import { describe, it } from 'vitest';
describe('outer', () => {
  describe('empty inner', () => {});
  it('ok', () => {});
  describe.todo('todo inner');
});
