// Module B — the other half of the cycle. B imports from A; because A was entered first,
// when B evaluates, A is only partially initialized. B must only *reference* A's bindings
// lazily (inside functions), like real model files do.
import { aName, AModel } from './a';

export const bName = 'B';

export class Base {
  tag() {
    return 'Base';
  }
}

export function describeA() {
  // referenced lazily — safe even though A wasn't finished when B's body ran.
  return `B sees: ${aName} via ${AModel.name}`;
}
