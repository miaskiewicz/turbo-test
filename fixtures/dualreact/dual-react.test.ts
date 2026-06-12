import * as React from 'react';

describe('single React across the setup-bundled ESM-dependency boundary', () => {
  it('the setup-bundled dep shares the one React module record (no dual-React)', () => {
    // Detect dual-React by behavior, not object identity (namespace wrappers differ):
    // set the dispatcher on the runtime React, then ask the SETUP-bundled dep's React to
    // read it. If the dep holds a second React copy, its dispatcher is still null.
    (React as any).__setDispatcher(null);
    (React as any).__setDispatcher({ live: true });
    const depUseRef = (globalThis as any).__depUseRefSetup as () => { current: string };
    // dual-React => dep's React.useRef throws "Cannot read properties of null (reading 'useRef')"
    expect(depUseRef()).toEqual({ current: 'dep' });
  });
});
