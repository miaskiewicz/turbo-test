const { cpSync } = require('node:fs');
cpSync('fixtures/identity', 'node_modules/identity-fixture', { recursive: true });
cpSync('fixtures/consumer', 'node_modules/consumer-fixture', { recursive: true });
