import { describe, expect, it } from 'vitest';

const SVG = 'http://www.w3.org/2000/svg';

// Elements in the SVG namespace implement ElementCSSInlineStyle like HTML ones (React sets
// inline styles on <svg> icons everywhere).
describe('SVG element inline style', () => {
  it('exposes a CSSStyleDeclaration that can be written and read', () => {
    const svg = document.createElementNS(SVG, 'svg');
    svg.style.flex = '1';
    expect(svg.style.flex).toBe('1');
    svg.style.setProperty('fill', 'red');
    expect(svg.style.getPropertyValue('fill')).toBe('red');
    expect(svg.style instanceof CSSStyleDeclaration).toBe(true);
  });

  it('reflects the style attribute both ways', () => {
    const path = document.createElementNS(SVG, 'path');
    path.setAttribute('style', 'stroke: blue');
    expect(path.style.stroke).toBe('blue');
    path.style.opacity = '0.5';
    expect(path.getAttribute('style')).toContain('opacity');
  });

  it('forwards `style = text` to cssText and is stable per element', () => {
    const g = document.createElementNS(SVG, 'g');
    g.style = 'fill: green';
    expect(g.style.fill).toBe('green');
    expect(g.style).toBe(g.style);
  });
});
