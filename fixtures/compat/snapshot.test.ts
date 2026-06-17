import { describe, it, expect } from 'vitest';

describe('snapshots', () => {
  it('matches a primitive snapshot', () => {
    expect(42).toMatchSnapshot();
  });

  it('matches an object snapshot', () => {
    expect({ b: 2, a: 1, nested: { z: [1, 2, 3] } }).toMatchSnapshot();
  });

  it('matches multiple snapshots in one test', () => {
    expect('first').toMatchSnapshot();
    expect('second').toMatchSnapshot();
  });

  it('matches an inline snapshot', () => {
    expect({ ok: true }).toMatchInlineSnapshot(`
      {
        "ok": true,
      }
    `);
  });

  it('matches a thrown error snapshot', () => {
    expect(() => { throw new Error('boom'); }).toThrowErrorMatchingSnapshot();
  });
});
