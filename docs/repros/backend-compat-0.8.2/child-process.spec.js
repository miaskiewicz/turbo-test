test('starts a Node subprocess', () => {
  const { spawnSync } = require('child_process');
  const result = spawnSync(process.execPath, ['-e', 'process.stdout.write("ok")'], {encoding: 'utf8'});
  expect(result.status).toBe(0);
  expect(result.stdout).toBe('ok');
});
