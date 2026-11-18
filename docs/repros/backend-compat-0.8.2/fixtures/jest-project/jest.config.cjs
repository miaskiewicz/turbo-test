module.exports = {
  testEnvironment: 'node',
  globalSetup: '<rootDir>/global-setup.cjs',
  globalTeardown: '<rootDir>/global-teardown.cjs',
  moduleNameMapper: { '^@fixture$': '<rootDir>/fixture-value.cjs' },
};
