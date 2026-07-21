import { describe, it, expect } from 'vitest';
import { theValue } from './src/value';

describe('module 26 uses jest-dom-ish matchers', () => {
  it('toBeCustom is registered from setupFiles', () => {
    expect(theValue()).toBeCustom();
  });
  it('toBeInTheDocumentish is registered from setupFiles', () => {
    expect({ node: 26 }).toBeInTheDocumentish();
  });
});
