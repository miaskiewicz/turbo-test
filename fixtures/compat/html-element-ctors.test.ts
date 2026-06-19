import { describe, it, expect } from 'vitest';

// The DOM bootstrap must register the full HTML*Element constructor surface — app bundles
// feature-detect / instanceof / subclass these, and an undefined reference aborts a chunk
// mid-hydration. Single-tag elements get a tag-keyed `instanceof`; the ctor name must be specific.

describe('extra HTML*Element constructors', () => {
  it('the constructor globals are all defined functions', () => {
    const names = [
      'HTMLDialogElement', 'HTMLDataListElement', 'HTMLFieldSetElement', 'HTMLLegendElement',
      'HTMLOListElement', 'HTMLDListElement', 'HTMLPreElement', 'HTMLTableRowElement',
      'HTMLTableCellElement', 'HTMLTableSectionElement', 'HTMLTableColElement',
      'HTMLTableCaptionElement', 'HTMLProgressElement', 'HTMLMeterElement', 'HTMLDetailsElement',
      'HTMLPictureElement', 'HTMLSourceElement', 'HTMLMediaElement', 'HTMLVideoElement',
      'HTMLAudioElement', 'HTMLTemplateElement', 'HTMLSlotElement', 'HTMLBodyElement',
      'HTMLHtmlElement', 'HTMLHeadElement', 'HTMLMetaElement', 'HTMLLinkElement', 'HTMLTitleElement',
      'HTMLBaseElement', 'HTMLBRElement', 'HTMLHRElement', 'HTMLOptGroupElement', 'HTMLMapElement',
      'HTMLAreaElement', 'HTMLObjectElement', 'HTMLEmbedElement', 'HTMLOutputElement',
      'HTMLQuoteElement', 'HTMLMenuElement', 'HTMLDataElement', 'HTMLTimeElement',
      'HTMLUnknownElement',
    ];
    for (const n of names) {
      expect(typeof (globalThis as any)[n], `${n} should be a function`).toBe('function');
      expect((globalThis as any)[n].name).toBe(n);
    }
  });

  it('tag-keyed instanceof matches the right element', () => {
    const dialog = document.createElement('dialog');
    const details = document.createElement('details');
    expect(dialog instanceof HTMLDialogElement).toBe(true);
    expect(details instanceof HTMLDetailsElement).toBe(true);
    // wrong interface must NOT match
    expect(dialog instanceof HTMLDetailsElement).toBe(false);
    // still an HTMLElement / Element / Node
    expect(dialog instanceof HTMLElement).toBe(true);
    expect(dialog instanceof Element).toBe(true);
  });

  it('node.constructor.name resolves to the specific interface', () => {
    expect(document.createElement('dialog').constructor.name).toBe('HTMLDialogElement');
    expect(document.createElement('progress').constructor.name).toBe('HTMLProgressElement');
    expect(document.createElement('td').constructor.name).toBe('HTMLTableCellElement');
    expect(document.createElement('tbody').constructor.name).toBe('HTMLTableSectionElement');
    expect(document.createElement('blockquote').constructor.name).toBe('HTMLQuoteElement');
  });
});
