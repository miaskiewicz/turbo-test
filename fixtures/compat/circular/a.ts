// Module A — part of a require cycle with B (mimics Sequelize/Mongoose models that
// reference each other). A imports a *value* from B and a class from B that it extends.
import { bName, Base } from './b';

export const aName = 'A';

// A reads B's exported value at MODULE-EVAL time (top-level), the classic circular read.
// Node/esbuild defer this through the live exports object; an eager `const` read TDZ-throws.
export const greetingFromB = `A sees: ${bName}`;

// A extends a class defined in B (Sequelize models extend Model, often cross-file).
export class AModel extends Base {
  who() {
    return 'AModel:' + bName;
  }
}
