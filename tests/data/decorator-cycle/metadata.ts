export const fieldTypes = new Map<object, Map<string, unknown>>();
(Reflect as any).metadata = (kind: string, value: unknown) => (target: object, key: string) => {
  if (kind !== 'design:type') return;
  let fields = fieldTypes.get(target);
  if (!fields) { fields = new Map(); fieldTypes.set(target, fields); }
  fields.set(key, value);
};
export function relation(_target: () => unknown): PropertyDecorator { return () => {}; }
export function entity<T extends Function>(constructor: T): T { return constructor; }
