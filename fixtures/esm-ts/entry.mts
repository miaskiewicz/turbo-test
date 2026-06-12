import { add, sum, Pair } from './math.ts';
const pair: Pair = { a: 4, b: 5 };
const r: number = add(0, sum(pair)); // 9
globalThis.__result = r;
log('ts esm evaluated, __result=' + r);
