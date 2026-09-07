import { describe, it, expect } from 'vitest';
// vite-plugin-svgr: `?react` yields a React component (default export) plus the legacy
// `ReactComponent` named export. turbo-test resolves both to a forwardRef <svg> stub that builds its
// element with React's own createElement. The fixture aliases `react` to a shim whose shapes mirror
// real React (forwardRef returns a `react.forward_ref` object exposing `.render(props, ref)`), so
// these assertions reflect real-React behavior, not a shim artifact.
import Icon, { ReactComponent } from './icon.svg?react';

describe('vite-plugin-svgr ?react import', () => {
  it('default export is a React forwardRef component', () => {
    expect((Icon as any).$$typeof).toBe(Symbol.for('react.forward_ref'));
    expect(typeof (Icon as any).render).toBe('function');
  });

  it('ReactComponent named export is the same component', () => {
    expect(ReactComponent).toBe(Icon);
  });

  it('renders an <svg> element with forwarded props', () => {
    const el: any = (Icon as any).render({ className: 'logo', 'data-testid': 'brand' }, null);
    expect(el.type).toBe('svg');
    expect(el.props.className).toBe('logo');
    expect(el.props['data-testid']).toBe('brand');
  });

  it('forwards a ref to the underlying <svg>', () => {
    const ref = { current: null };
    const el: any = (Icon as any).render({}, ref);
    expect(el.props.ref).toBe(ref);
  });

  it('defaults aria-hidden when no accessible name/role is given', () => {
    const el: any = (Icon as any).render({}, null);
    expect(el.props['aria-hidden']).toBe(true);
  });
});
