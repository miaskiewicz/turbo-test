const __blockListRules = new WeakMap();
function __blockListGet(owner) {
  const rules = __blockListRules.get(owner);
  if (!rules) throw new TypeError('Invalid BlockList receiver');
  return rules;
}
function __blockListFamily(type = 'ipv4') {
  if (typeof type !== 'string') throw new TypeError('family must be a string');
  return type.toLowerCase();
}
function __blockListNative(request) {
  const response = JSON.parse(globalThis.__ttBlockList(JSON.stringify(request)));
  if (response.error) throw Object.assign(new Error(response.error), { code: 'ERR_INVALID_ARG_VALUE' });
  return response.value;
}
function __blockListAdd(owner, request) {
  const rules = __blockListGet(owner);
  rules.unshift(__blockListNative(request));
}
class __NodeBlockList {
  constructor() { __blockListRules.set(this, []); }
  static isBlockList(value) { return __blockListRules.has(value); }
  addAddress(address, type) { __blockListAdd(this, { kind: 'address', address, family: __blockListFamily(type) }); }
  addRange(start, end, type) { __blockListAdd(this, { kind: 'range', address: start, end, family: __blockListFamily(type) }); }
  addSubnet(address, prefix, type) {
    if (!Number.isInteger(prefix)) throw new TypeError('prefix must be an integer');
    __blockListAdd(this, { kind: 'subnet', address, prefix, family: __blockListFamily(type) });
  }
  check(address, type) {
    const rules = __blockListGet(this), family = __blockListFamily(type);
    if (typeof address !== 'string') throw new TypeError('address must be a string');
    return __blockListNative({ kind: 'check', address, family, rules });
  }
  get rules() { return __blockListGet(this).map((rule) => rule.label); }
}
