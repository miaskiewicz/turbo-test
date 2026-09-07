// Minimal stand-in for `react` so the svgr fixture is self-contained (no installed react needed).
// The `?react` stub imports `createElement` + `forwardRef` from 'react'; this fixture aliases
// 'react' to this shim (see vitest.config.ts). It mimics enough of React's element/forwardRef shape
// to assert the stub forwards props + ref and builds an <svg> element.
export function createElement(type: string, props: Record<string, unknown> | null) {
  return { $$typeof: Symbol.for('react.element'), type, props: props || {}, ref: (props && (props as any).ref) ?? null };
}
export function forwardRef(render: (props: any, ref: any) => unknown) {
  const Comp = (props: any) => render(props, (props && props.ref) ?? null);
  (Comp as any).$$typeof = Symbol.for('react.forward_ref');
  return Comp;
}
export default { createElement, forwardRef };
