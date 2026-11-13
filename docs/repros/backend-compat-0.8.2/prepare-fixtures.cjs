const { cpSync } = require('node:fs');
cpSync('fixtures/identity', 'node_modules/identity-fixture', { recursive: true });
cpSync('fixtures/consumer', 'node_modules/consumer-fixture', { recursive: true });
cpSync('fixtures/jest-config-edge/conditional-package', 'fixtures/jest-config-edge/node_modules/conditional-fixture', { recursive: true });
