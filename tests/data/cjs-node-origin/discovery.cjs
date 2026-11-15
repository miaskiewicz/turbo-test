exports.getCallerFile = function getCallerFile() {
  const previous = Error.prepareStackTrace;
  const limit = Error.stackTraceLimit;
  try {
    Error.stackTraceLimit = 10;
    Error.prepareStackTrace = (error, frames) => frames;
    const captured = {};
    Error.captureStackTrace(captured);
    for (const frame of captured.stack) {
      const filename = frame.getFileName();
      if (filename !== __filename) return filename;
    }
  } finally {
    Error.prepareStackTrace = previous;
    Error.stackTraceLimit = limit;
  }
};
