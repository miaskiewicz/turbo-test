import { describe, it, test } from 'vitest';
describe('only skips', () => { it.skip('s1', () => {}); it.skip('s2', () => {}); });
describe('only todos', () => { it.todo('t1'); });
describe.todo('todo suite');
describe('mixed', () => { it.skip('s', () => {}); it('p', () => {}); });
test('runtime skip', ({ skip }) => { skip('not now'); });
describe('nested skip parent', () => { describe('child', () => { it.skip('x', () => {}); }); });
