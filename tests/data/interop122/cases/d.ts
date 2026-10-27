// Describes an imported value for the interop table (turbo-test#122): functions by name,
// objects by their sorted own enumerable keys (two levels) plus an `!esm` marker for a
// truthy `__esModule`, primitives by type and value.
import { expect, it } from 'vitest';
import { expected } from './expected';

export function d(v: unknown, depth = 0): string {
  if (v === null) return 'null';
  const t = typeof v;
  if (t === 'function') return `fn:${(v as Function).name}`;
  if (t !== 'object') return `${t}:${String(v)}`;
  if (depth > 1) return 'obj';
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o).sort();
  return `{${keys.map((k) => `${k}=${d(o[k], depth + 1)}`).join(',')}}${o.__esModule ? '!esm' : ''}`;
}

/** One table row: `value()` is described and compared with vitest 4.1.12's description. */
export function row(file: string, name: string, value: () => unknown) {
  it(name, async () => {
    let got: string;
    try {
      got = d(await value());
    } catch (e) {
      got = `throws:${(e as Error).message.split('\n')[0]}`;
    }
    console.log(`ROW ${file}|${name}|${got}`);
    // the message carries the whole description (chai truncates long values)
    expect(got, `GOT<${got}>`).toBe(expected[`${file}|${name}`]);
  });
}
