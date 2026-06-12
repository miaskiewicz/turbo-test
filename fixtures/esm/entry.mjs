import { add } from './math.mjs';
import { TWO, THREE } from './const.mjs';
import { twice } from './math.mjs';
// shared dep (const.mjs) imported via two paths -> must be the SAME instance
globalThis.__result = add(TWO, THREE) + twice; // 5 + 4 = 9
log('entry evaluated, __result=' + globalThis.__result);
