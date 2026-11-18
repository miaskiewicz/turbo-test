const fs = require('fs'), path = require('path'), ts = require('typescript');
const config = ts.getParsedCommandLineOfConfigFile(path.join(__dirname, 'tsconfig.json'), {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic(d) { throw new Error(ts.flattenDiagnosticMessageText(d.messageText, '\n')); } });
require.extensions['.ts'] = (module, fileName) => module._compile(ts.transpileModule(fs.readFileSync(fileName, 'utf8'), { fileName, compilerOptions: config.options }).outputText, fileName);
require('./check.test.ts');
