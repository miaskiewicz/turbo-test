module.exports = {
  testEnvironment: 'node',
  injectGlobals: false,
  testMatch: ['**/included.spec.js'],
  setupFilesAfterEnv: ['<rootDir>/setup.cjs'],
};
