import { describe, it, expect } from 'vitest';

describe('silent group', () => {
  it('logs to console but still passes', () => {
    console.log('TURBO_SILENT_MARKER_LOG');
    console.info('TURBO_SILENT_MARKER_INFO');
    console.warn('TURBO_SILENT_MARKER_WARN');
    console.error('TURBO_SILENT_MARKER_ERROR');
    expect(1).toBe(1);
  });
});
