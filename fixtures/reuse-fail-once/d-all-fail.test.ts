import { appendFileSync, existsSync } from 'node:fs';
import { expect, it } from 'vitest';
// turbo-test#144: a file whose every test genuinely fails. Every evaluation is logged (when the
// test created runs.log), so the test can count runs.
const log = `${process.cwd()}/runs.log`;
if (existsSync(log)) appendFileSync(log, 'd-all-fail\n');
it('fails', () => expect('a').toBe('b'));
