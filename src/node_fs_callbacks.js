// ---- fs callback API (turbo-test#433) ----
// fs.readFile / stat / open / read / ... (cb) over the same sync implementations as fs.*Sync and
// fs.promises: an argument error throws at the call as in Node, every other outcome reaches the
// callback later, from the event loop (a real-clock internal macrotask: after the current tick,
// its nextTicks and promise jobs; never moved by fake timers), with Node's arguments.
function __fsCallbackArg(args) {
  const callback = args.pop();
  if (typeof callback !== 'function') throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'The "cb" argument must be of type function.' + __usageReceived(callback));
  return callback;
}
function __fsLater(callback, args) {
  const run = __captureAsyncContext(() => callback(...args));
  __scheduleInternal(() => { try { run(); } finally { __loop.drainNextTicks(); } }, 0);
}
// Node validates arguments before the operation starts: those errors throw synchronously; an
// ERR_FS_* (rm's EISDIR) is an operation result
const __fsThrowsNow = (error) => typeof (error && error.code) === 'string' && /^ERR_(?!FS_)/.test(error.code);
function __fsPathArg(value) {
  if (typeof value === 'string' || Buffer.isBuffer(value) || value instanceof URL) return;
  throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'The "path" argument must be of type string or an instance of Buffer or URL.' + __usageReceived(value));
}
function __fsSettle(operation, shape) {
  try { return [null, ...shape(operation())]; }
  catch (error) { if (__fsThrowsNow(error)) throw error; return [error]; }
}
const __cbNone = () => [];
const __cbValue = (value) => [value];
const __cbOptional = (value) => (value === undefined ? [] : [value]);
// fs.<name>(...args, cb) from its sync form; `path` checks the first argument's type first
function __fsCallbackForm(sync, shape, path) {
  return function (...args) {
    const callback = __fsCallbackArg(args);
    if (path) __fsPathArg(args[0]);
    __fsLater(callback, __fsSettle(() => sync(...args), shape));
  };
}
// fs.close(fd[, cb]): without a callback a failure is thrown from the event loop, as in Node
function __fsClose(fd, callback = (error) => { if (error) throw error; }) {
  __fsLater(callback, __fsSettle(() => __fs.closeSync(fd), __cbNone));
}
// fs.exists(path, cb): cb(boolean), no error argument
function __fsExists(path, callback) {
  __fsLater(__fsCallbackArg([callback]), [__fs.existsSync(path)]);
}
const __fsFdResult = (key) => (result) => [result[key], result.buffer];
function __fsInstallCallbacks() {
  const path = true;
  const forms = {
    readFile: [__readFile, __cbValue, path], writeFile: [__fs.writeFileSync, __cbNone, path], appendFile: [__fs.appendFileSync, __cbNone, path],
    mkdir: [__mkdir, __cbOptional, path], rm: [__rm, __cbNone, path], rmdir: [__fs.rmdirSync, __cbNone, path], unlink: [__fs.unlinkSync, __cbNone, path],
    stat: [__fs.statSync, __cbValue, path], lstat: [__fs.lstatSync, __cbValue, path], readdir: [__readdir, __cbValue, path],
    realpath: [__realpathSync, __cbValue, path], symlink: [__fs.symlinkSync, __cbNone, path], access: [__accessSync, __cbNone, path],
    chmod: [__chmodSync, __cbNone, path], rename: [__renameSync, __cbNone, path], open: [__openSync, __cbValue, path],
    read: [(fd, ...a) => __fdRead(__fdChecked(fd), ...a), __fsFdResult('bytesRead')], write: [(fd, ...a) => __fdWrite(__fdChecked(fd), ...a), __fsFdResult('bytesWritten')],
    fstat: [__fs.fstatSync, __cbValue], ftruncate: [__fs.ftruncateSync, __cbNone], fsync: [__fs.fsyncSync, __cbNone],
    fdatasync: [__fs.fdatasyncSync, __cbNone], fchmod: [__fs.fchmodSync, __cbNone],
  };
  for (const [name, [sync, shape, checkPath]] of Object.entries(forms)) __fs[name] = __fsCallbackForm(sync, shape, checkPath);
  __fs.realpath.native = __fsCallbackForm(__realpathNative, __cbValue, path);
  __fs.close = __fsClose;
  __fs.exists = __fsExists;
}
__fsInstallCallbacks();
