import { describe, it, expect } from 'vitest';

// issue #18: a top-level (non-framed) window must self-reference the way browsers, jsdom, and
// vitest do. `window.parent !== window` is the canonical "am I inside an iframe?" check; when the
// framing globals are undefined it reads `true`, so top-level detection breaks silently.

describe('top-level window self-references', () => {
  it('parent / top / self / frames === window', () => {
    expect(window.parent).toBe(window);
    expect(window.top).toBe(window);
    expect(window.self).toBe(window);
    expect((window as any).frames).toBe(window);
  });

  it('frameElement is null for a top-level window', () => {
    expect(window.frameElement).toBe(null);
  });

  it('the iframe-detection guard reports NOT framed', () => {
    expect(window.parent !== window).toBe(false);
    expect(window.top !== window).toBe(false);
  });

  it('framing can still be stubbed (globals are configurable)', () => {
    const fakeParent = {} as Window;
    Object.defineProperty(window, 'parent', { value: fakeParent, configurable: true });
    expect(window.parent).toBe(fakeParent);
    expect(window.parent !== window).toBe(true);
    // restore so we don't leak into other assertions
    Object.defineProperty(window, 'parent', { value: window, configurable: true });
    expect(window.parent).toBe(window);
  });
});
