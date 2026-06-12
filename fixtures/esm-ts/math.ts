export const add = (a: number, b: number): number => a + b;
export interface Pair { a: number; b: number }
export const sum = (p: Pair): number => add(p.a, p.b);
