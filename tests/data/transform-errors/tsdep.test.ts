import { test } from 'vitest';
import { x } from './dep.ts';
test('t', () => { x; });
