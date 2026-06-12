import lib from '../cjs/lib.cjs';            // ESM imports CJS default (= module.exports)
import { VERSION } from '../cjs/lib.cjs';    // ESM imports CJS named export
import consumer from '../cjs/consumer.cjs';  // pulls in CJS->CJS require chain
globalThis.__out = lib.greet('esm') + '|' + VERSION + '|' + consumer.combined;
log(globalThis.__out);
