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

Additional runtime probes cover async context, hashing, gzip, abort timeouts,
data URL fetch, structured cloning, Web Crypto, IP parsing, EventEmitter,
diagnostics channels, V8 serialization, FormData and callback stream pipelines.

  npx turbo-test --jobs 1 additional-runtime.spec.js more-runtime.spec.js
  npx jest --runInBand additional-runtime.spec.js more-runtime.spec.js

Jest passes all 15 assertions. Turbo passes the Map and ArrayBuffer clone
control and reproduces the 14 remaining failures.

The coverage test passes under both runners. Turbo includes only src/used.js
and exits 0 at 100% coverage. Jest also includes src/unused.js, reports 50%
line coverage, and exits 1 at the same threshold:

  npx turbo-test --jobs 1 --coverage --coverage-include 'src/*.js' --coverage-thresholds lines=100 coverage.spec.js
  npx jest --runInBand --coverage --collectCoverageFrom 'src/*.js' --coverageThreshold '{"global":{"lines":100}}' coverage.spec.js

Backend Jest configuration and API probes

Run from the fixture directory so the launchers discover its own Jest config.
Start without a teardown marker.

  cd fixtures/jest-project
  rm -f .teardown-ran
  ../../node_modules/.bin/turbo-test --jobs 1 backend-config.spec.js jest-api.spec.js
  test ! -e .teardown-ran
  ../../node_modules/.bin/jest --runInBand backend-config.spec.js jest-api.spec.js
  cat .teardown-ran
  rm -f .teardown-ran

Jest passes all five assertions and writes "closed" during globalTeardown.
Turbo fails all five assertions and leaves no marker. The probes cover
globalSetup, globalTeardown, moduleNameMapper, isolateModulesAsync,
restoreAllMocks with replaceProperty, and vm.runInNewContext.

Additional Jest API controls

From fixtures/jest-project, compare the public mock and timer APIs:

  ../../node_modules/.bin/turbo-test --jobs 1 jest-surface.spec.js
  ../../node_modules/.bin/jest --runInBand jest-surface.spec.js
  rm -f .teardown-ran

Jest passes all ten tests. Turbo passes seven, including factory and virtual
mocks, local automock, resetAllMocks, restoring a spy, fake timer advancement
and resetModules. Turbo fails requireActual error propagation, automatic
mock generation in requireMock, and flushing process.nextTick in runAllTicks.

Jest discovery and explicit globals

From the kit root, enter fixtures/jest-discovery. Run both commands without
test path arguments to exercise discovery:

  cd fixtures/jest-discovery
  ../../node_modules/.bin/turbo-test --jobs 1
  ../../node_modules/.bin/jest --runInBand

Both runners execute setupFilesAfterEnv and load @jest/globals. Jest passes
both assertions in the included file. Turbo injects globals despite
injectGlobals: false and runs the file that testMatch excludes.
