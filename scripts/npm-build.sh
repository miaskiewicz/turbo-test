#!/usr/bin/env bash
# Build the native turbo-test binary and place it where cli.js expects it:
#   bin/turbo-test-<platform>-<arch>   (Node's process.platform / process.arch naming)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --bin turbo-test
mkdir -p bin
node -e '
  const fs = require("fs");
  const dst = `bin/turbo-test-${process.platform}-${process.arch}`;
  fs.copyFileSync("target/release/turbo-test", dst);
  fs.chmodSync(dst, 0o755);
  console.log("placed " + dst);
'
