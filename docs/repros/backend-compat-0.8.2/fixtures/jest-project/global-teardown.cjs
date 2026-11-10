const fs = require('node:fs');
const path = require('node:path');

module.exports = async () => {
  fs.writeFileSync(path.join(__dirname, '.teardown-ran'), 'closed');
};
