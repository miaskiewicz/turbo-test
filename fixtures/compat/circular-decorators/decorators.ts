// Minimal stand-ins for sequelize-typescript / NestJS class+property decorators. They run at
// class-definition time (like @Table / @Column / @Injectable), which is exactly when a
// mis-lowered (standard-semantics or un-lowered) decorator would TDZ-throw.
export function Entity(name: string) {
  return function <T extends { new (...args: any[]): object }>(target: T): T {
    (target as any).entityName = name;
    return target;
  };
}

export function Field(opts: { type: string }) {
  return function (_target: object, key: string) {
    void opts;
    void key;
  };
}
