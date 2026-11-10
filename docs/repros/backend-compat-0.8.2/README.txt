Turbo Test 0.8.2 backend compatibility reproductions

Run these commands from this directory. The repros use public packages and were
verified with Node 22.23.3 on macOS arm64. The HTTP and subprocess controls
require a shell that permits loopback sockets and child processes.

  npm ci
  node prepare-fixtures.cjs

Turbo Test reproduces all seven failing assertions across these six suites:

  npx turbo-test --jobs 1 identity.spec.js resolver.spec.js crypto.spec.js streams.spec.js http-socket.spec.js child-process.spec.js

Node 22 confirms the identity assertion with native require(esm):

  node -e "require('node:assert/strict').strictEqual(require('identity-fixture').state, require('identity-fixture/state').state)"

Jest passes the six assertions in the remaining five suites:

  npx jest --runInBand resolver.spec.js crypto.spec.js streams.spec.js http-socket.spec.js child-process.spec.js

The coverage test passes under both runners. Turbo includes only src/used.js
and exits 0 at 100% coverage. Jest also includes src/unused.js, reports 50%
line coverage, and exits 1 at the same threshold:

  npx turbo-test --jobs 1 --coverage --coverage-include 'src/*.js' --coverage-thresholds lines=100 coverage.spec.js
  npx jest --runInBand --coverage --collectCoverageFrom 'src/*.js' --coverageThreshold '{"global":{"lines":100}}' coverage.spec.js
