// Minimal stand-in for `react` so the svgr fixture is self-contained (no installed react needed).
// The `?react` stub imports `createElement` + `forwardRef` from 'react'; this fixture aliases
// 'react' to this shim (see vitest.config.ts). It mirrors REAL React's shapes: `createElement`
// returns a `react.element`, and `forwardRef` returns a `react.forward_ref` OBJECT (not a callable)
// exposing `.render(props, ref)` — so the fixture asserts against the same shape real React yields,
// not a shim-only artifact.
export function createElement(type: string, props: Record<string, unknown> | null) {
  const p = props || {};
  return { $$typeof: Symbol.for('react.element'), type, props: p, ref: (p as any).ref ?? null };
}
export function forwardRef(render: (props: any, ref: any) => unknown) {
  return { $$typeof: Symbol.for('react.forward_ref'), render };
}
export default { createElement, forwardRef };
