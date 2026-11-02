import { appendFileSync, existsSync } from 'node:fs';
import { expect, it } from 'vitest';
// turbo-test#126: every evaluation of this file is logged (when the test created runs.log), so the test can count runs.
const log = `${process.cwd()}/runs.log`;
if (existsSync(log)) appendFileSync(log, 'c-pass\n');
it('passes', () => expect(1 + 1).toBe(2));
