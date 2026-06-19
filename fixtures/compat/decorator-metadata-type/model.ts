import { Shape } from './shape.types';
function Col(): PropertyDecorator { return () => {}; }
export class Model {
  @Col() declare data: Shape;          // emitDecoratorMetadata → design:type references Shape (an interface!)
  @Col() declare name: string;
}
