// Jest 30 fails this file to load with `Cannot access 'mockGet' before initialization`:
// the mock-prefixed const is only declared below the import of `./decorating`, which is the
// first require (the hoist moves imports to the module's top). Jest's factory, hoisted above
// the imports, runs lazily on the first `require('./x')` inside `./decorating`, and by then
// `mockGet` is still in its temporal dead zone. turbo-test must match.
import { Something } from './decorating';
const mockGet = jest.fn();
jest.mock('./x', () => ({ get: mockGet }));

test('never runs: file fails to load with TDZ', () => {
  mockGet.mockReturnValue('mocked');
  expect(Something('k')).toBe('mocked');
});
