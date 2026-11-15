const discovery = require('caller-discovery');
module.exports = { caller: discovery.getCallerFile(), filename: __filename };
