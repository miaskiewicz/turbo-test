import { describe, it, expect } from 'vitest';

// emotion/MUI construct stylesheets then adopt them:
//   const s = new CSSStyleSheet(); document.adoptedStyleSheets = [...document.adoptedStyleSheets, s];
// No layout engine here — the rule list is an inert store that satisfies the API surface so the
// chunk doesn't abort mid-hydration.

describe('constructable CSSStyleSheet + adoptedStyleSheets', () => {
  it('new CSSStyleSheet() exposes cssRules/rules', () => {
    const s = new CSSStyleSheet();
    expect(Array.isArray(s.cssRules)).toBe(true);
    expect(s.rules).toBe(s.cssRules);
  });

  it('insertRule appends and returns the index; deleteRule removes', () => {
    const s = new CSSStyleSheet();
    const i0 = s.insertRule('.a { color: red }');
    expect(i0).toBe(0);
    const i1 = s.insertRule('.b { color: blue }');
    expect(i1).toBe(1);
    expect(s.cssRules.length).toBe(2);
    s.deleteRule(0);
    expect(s.cssRules.length).toBe(1);
    expect(s.cssRules[0].cssText).toBe('.b { color: blue }');
  });

  it('replace returns a Promise<this>; replaceSync is synchronous', async () => {
    const s = new CSSStyleSheet();
    const r = s.replace('.x { display: none }');
    expect(typeof r.then).toBe('function');
    await expect(r).resolves.toBe(s);
    s.replaceSync('.y { display: block }');
    expect(s.cssRules.length).toBe(1);
    expect(s.cssRules[0].cssText).toBe('.y { display: block }');
  });

  it('document.adoptedStyleSheets is a settable array (read-modify-write spread)', () => {
    expect(Array.isArray(document.adoptedStyleSheets)).toBe(true);
    const s = new CSSStyleSheet();
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, s];
    expect(document.adoptedStyleSheets[document.adoptedStyleSheets.length - 1]).toBe(s);
  });
});
