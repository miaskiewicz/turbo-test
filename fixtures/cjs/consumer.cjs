const lib = require('./lib.cjs');           // CJS requires CJS
module.exports.combined = lib.greet('cjs') + ' v' + lib.VERSION;
