// fs.watch receives real OS events from a context-owned native watcher.
function __watchNative(operation, args) {
  const result = JSON.parse(globalThis.__ttFsWatch(operation, JSON.stringify(args)));
  if (result.error) throw Object.assign(new Error(result.error.message), result.error);
  return result.value;
}
class __FSWatcher extends __EventEmitter {
  constructor(path, options) {
    super();
    this._path = path;
    this._encoding = options.encoding || 'utf8';
    if (!['utf8', 'utf-8', 'buffer'].includes(this._encoding)) throw __fsArgumentError('ERR_INVALID_ARG_VALUE', 'Unsupported fs.watch encoding');
    if (options.persistent === false) throw __fsArgumentError('ERR_TURBO_UNSUPPORTED_FS_WATCH', 'Non-persistent fs.watch is not supported', Error);
    this._id = __watchNative('open', { path, recursive: !!options.recursive });
    this._timer = __realTimers.setInterval(__captureAsyncContext(() => this._poll()), 10);
    if (options.signal) {
      if (options.signal.aborted) this.close();
      else options.signal.addEventListener('abort', () => this.close(), { once: true });
    }
  }
  _poll() {
    try {
      for (const item of __watchNative('poll', { id: this._id })) {
        if (this._id === null) break;
        const name = item.filename === null || this._encoding !== 'buffer' ? item.filename : Buffer.from(item.filename);
        this.emit('change', item.event, name);
      }
    } catch (error) { this.close(); this.emit('error', error); }
  }
  close() {
    if (this._id === null) return;
    __realTimers.clearInterval(this._timer);
    __watchNative('close', { id: this._id });
    this._id = null;
    queueMicrotask(() => this.emit('close'));
  }
  ref() { return this; }
  unref() { throw __fsArgumentError('ERR_TURBO_UNSUPPORTED_FS_WATCH', 'Unref for fs.watch is not supported', Error); }
}
function __watch(path, options, listener) {
  if (typeof options === 'function') { listener = options; options = {}; }
  else if (typeof options === 'string') options = { encoding: options };
  options ||= {};
  const file = __fsCheckedPath(path, 'filename');
  __fs.statSync(file);
  const watcher = new __FSWatcher(file, options);
  if (listener !== undefined) watcher.on('change', listener);
  return watcher;
}
Object.assign(__fs, { watch: __watch, FSWatcher: __FSWatcher });
