import { TWO } from './const.mjs';
export const add = (a, b) => a + b;
export const double = (x) => add(x, x);
export const twice = double(TWO); // 4
