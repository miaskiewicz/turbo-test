// A property decorator that needs emitDecoratorMetadata's `design:type` at class-definition time
// (the Mongoose @Prop / Sequelize @Column shape). Without metadata it THROWS at load, so
// turbo-test's retry-on-load re-transforms this file through the metadata path (project tsc, or
// oxc fallback) where design:type IS emitted — then the decorator runs and the module loads.
function RequireType(target: any, key: string) {
  const t = Reflect.getMetadata('design:type', target, key);
  if (typeof t !== 'function') throw new Error('no design:type for ' + key);
  (target.constructor.__fieldTypes || (target.constructor.__fieldTypes = {}))[key] = t.name;
}

class Model {
  @RequireType count!: number;
  @RequireType label!: string;
  @RequireType active!: boolean;
}

describe('emitDecoratorMetadata (retry-on-load)', () => {
  it('emits design:type so a metadata-reading decorator runs', () => {
    const types = (Model as any).__fieldTypes;
    expect(types).toEqual({ count: 'Number', label: 'String', active: 'Boolean' });
  });
});
