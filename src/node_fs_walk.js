// Filesystem copy and glob use the native fs seam, retaining per-isolate cwd.
function __cpOptions(options) {
  if (options == null) return {};
  if (typeof options !== 'object') throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'options must be an object');
  if (options.preserveTimestamps) throw __fsArgumentError('ERR_TURBO_UNSUPPORTED_FS_CP', 'Requested fs.cpSync option is not supported', Error);
  return options;
}
function __cpFile(source, destination, options) {
  if (__fs.existsSync(destination) && options.force === false) {
    if (options.errorOnExist) throw __fsArgumentError('ERR_FS_CP_EEXIST', 'Destination already exists: ' + destination, Error);
    return;
  }
  __copyFileSync(source, destination, options.mode);
}
function __cpDirectory(source, destination, options, ancestors) {
  if (!options.recursive) throw __fsArgumentError('ERR_FS_EISDIR', 'Cannot copy a directory without recursive: true', Error);
  __mkdir(destination, { recursive: true });
  for (const name of __readdir(source)) __cpEntry(__path.join(source, name), __path.join(destination, name), options, ancestors);
}
function __cpLink(source, destination, options) {
  if (__fs.lstatSync(destination, { throwIfNoEntry: false })) {
    if (options.force === false) {
      if (options.errorOnExist) throw __fsArgumentError('ERR_FS_CP_EEXIST', 'Destination already exists: ' + destination, Error);
      return;
    }
    __fs.unlinkSync(destination);
  }
  const target = globalThis.__fs_readlink(__fsPath(source));
  __fs.symlinkSync(options.verbatimSymlinks ? target : __path.resolve(__path.dirname(source), target), destination);
}
function __cpCheckedDirectory(source, destination, options, ancestors) {
  const real = globalThis.__fs_realpath(__fsPath(source));
  if (ancestors.has(real)) throw __fsArgumentError('ERR_FS_CP_EINVAL', 'Cannot copy a cyclic directory', Error);
  ancestors.add(real);
  try { __cpDirectory(source, destination, options, ancestors); } finally { ancestors.delete(real); }
}
function __cpEntry(source, destination, options, ancestors) {
  if (options.filter && !options.filter(source, destination)) return;
  let stat = __fs.lstatSync(source);
  if (stat.isSymbolicLink()) {
    if (!options.dereference) { __cpLink(source, destination, options); return; }
    stat = __fs.statSync(source);
  }
  if (stat.isDirectory()) __cpCheckedDirectory(source, destination, options, ancestors);
  else if (stat.isFile()) __cpFile(source, destination, options);
  else throw __fsArgumentError('ERR_TURBO_UNSUPPORTED_FS_CP', 'Copying this file type is not supported', Error);
}
function __cpSync(source, destination, options) {
  const src = __fsCheckedPath(source, 'src'), dest = __fsCheckedPath(destination, 'dest');
  const from = __path.resolve(src), to = __path.resolve(dest);
  if (to === from || to.startsWith(from + '/')) throw __fsArgumentError('ERR_FS_CP_EINVAL', 'Cannot copy a directory into itself', Error);
  __cpEntry(src, dest, __cpOptions(options), new Set());
}
function __globPrefix(pattern) {
  const index = pattern.search(/[?*{[]/);
  if (index < 0) return __path.dirname(pattern);
  const head = pattern.slice(0, index), slash = head.lastIndexOf('/');
  return slash < 0 ? '.' : head.slice(0, slash) || '/';
}
function __globAllowed(pattern, relative, options) {
  const explicit = pattern.split('/').filter(part => part.startsWith('.'));
  if (relative.split('/').some(part => part.startsWith('.') && !explicit.some(p => globalThis.__ttFsGlobMatch(p, part)))) return false;
  const excluded = options.exclude;
  if (typeof excluded === 'function' && excluded(relative)) return false;
  if (Array.isArray(excluded) && excluded.some(glob => globalThis.__ttFsGlobMatch(String(glob), relative))) return false;
  return globalThis.__ttFsGlobMatch(pattern, relative);
}
function __globReadDirectory(dir) {
  try { return __readdir(dir, { withFileTypes: true }); }
  catch (error) { if (error.code === 'ENOENT' || error.code === 'ENOTDIR') return []; throw error; }
}
function __globCanDescend(pattern, path) {
  if (pattern.includes('**') || pattern.includes('{')) return true;
  return path.split('/').filter(Boolean).length < pattern.split('/').filter(Boolean).length;
}
function __globDirectory(base, dir, pattern, options, out) {
  for (const entry of __globReadDirectory(dir)) {
    const full = __path.join(dir, entry.name), relative = pattern.startsWith('/') ? full : __path.relative(base, full);
    if (__globAllowed(pattern, relative, options)) out.set(relative, options.withFileTypes ? entry : relative);
    if (entry.isDirectory() && __globCanDescend(pattern, relative)) __globDirectory(base, full, pattern, options, out);
  }
}
function __globAddPattern(value, base, options, out) {
  if (typeof value !== 'string') throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'pattern must be a string');
  if (/[!+@?*]\(/.test(value)) throw __fsArgumentError('ERR_TURBO_UNSUPPORTED_FS_GLOB', 'Extglobs are not supported', Error);
  __globDirectory(base, __path.resolve(base, __globPrefix(value)), value, options, out);
}
function __globSync(pattern, options = {}) {
  if (options == null || typeof options !== 'object') throw __fsArgumentError('ERR_INVALID_ARG_TYPE', 'options must be an object');
  const patterns = Array.isArray(pattern) ? pattern : [pattern], out = new Map(), base = __path.resolve(options.cwd || '.');
  for (const value of patterns) __globAddPattern(value, base, options, out);
  return [...out.values()];
}
Object.assign(__fs, { cpSync: __cpSync, globSync: __globSync });
