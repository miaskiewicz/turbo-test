import { describe, it, expect } from 'vitest';
// vite-plugin-svgr: `?react` yields a React component (default export), plus the legacy
// `ReactComponent` named export. turbo-test resolves both to a render-safe <svg> stub.
import Icon, { ReactComponent } from './icon.svg?react';

const REACT_ELEMENT = Symbol.for('react.element');

describe('vite-plugin-svgr ?react import', () => {
  it('default export is a React function component', () => {
    expect(typeof Icon).toBe('function');
  });

  it('ReactComponent named export is the same component', () => {
    expect(ReactComponent).toBe(Icon);
  });

  it('rendering yields a real <svg> React element with forwarded props', () => {
    const el: any = Icon({ className: 'logo', 'data-testid': 'brand' });
    expect(el.$$typeof).toBe(REACT_ELEMENT);
    expect(el.type).toBe('svg');
    expect(el.props.className).toBe('logo');
    expect(el.props['data-testid']).toBe('brand');
  });

  it('defaults aria-hidden when no accessible name/role is given', () => {
    const el: any = Icon({});
    expect(el.props['aria-hidden']).toBe(true);
  });
});
