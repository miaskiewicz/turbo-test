import { appendFileSync, existsSync } from 'node:fs';
import { expect, it } from 'vitest';
// turbo-test#126: a file that genuinely fails — on any isolate, in any order. Every evaluation
// is logged (when the test created runs.log), so the test can count runs.
const log = `${process.cwd()}/runs.log`;
if (existsSync(log)) appendFileSync(log, 'b-fail\n');
it('passes', () => expect(2 * 2).toBe(4));
it('fails', () => expect(1 + 1).toBe(3));
