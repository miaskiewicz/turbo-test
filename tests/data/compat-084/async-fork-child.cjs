process.on('message', (message) => {
  if (message.kind === 'finish') { process.disconnect(); return; }
  process.send({ value: message.value, pid: process.pid, marker: process.env.ASYNC_PROCESS_MARKER });
});
process.on('disconnect', () => { process.exitCode = 0; });
