module.exports = {
  testEnvironment: 'node',
  testEnvironmentOptions: { customExportConditions: ['fixture-custom'] },
  setupFiles: ['<rootDir>/early.cjs'],
  setupFilesAfterEnv: ['<rootDir>/late.cjs'],
  resetMocks: true,
  restoreMocks: true,
  moduleDirectories: ['node_modules', 'custom-modules'],
  testTimeout: 35,
};
