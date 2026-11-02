"use strict";

Object.defineProperty(exports, "__esModule", {
  value: true
});
exports.viaStar = exports.viaPlain = exports.viaDefault = exports.named = exports.default = void 0;
var _expDefaultFlag = _interopRequireDefault(require("exp-default-flag"));
var Star = _interopRequireWildcard(require("mexp-fn"));
var _mexpFn = _interopRequireDefault(require("mexp-fn"));
function _interopRequireWildcard(e, t) { if ("function" == typeof WeakMap) var r = new WeakMap(), n = new WeakMap(); return (_interopRequireWildcard = function (e, t) { if (!t && e && e.__esModule) return e; var o, i, f = { __proto__: null, default: e }; if (null === e || "object" != typeof e && "function" != typeof e) return f; if (o = t ? n : r) { if (o.has(e)) return o.get(e); o.set(e, f); } for (const t in e) "default" !== t && {}.hasOwnProperty.call(e, t) && ((i = (o = Object.defineProperty) && Object.getOwnPropertyDescriptor(e, t)) && (i.get || i.set) ? o(f, t, i) : f[t] = e[t]); return f; })(e, t); }
function _interopRequireDefault(e) { return e && e.__esModule ? e : { default: e }; }
function BabelComp() {}
var _default = exports.default = BabelComp;
const named = exports.named = 'n';
const viaDefault = exports.viaDefault = _expDefaultFlag.default;
const viaStar = exports.viaStar = Star;
const viaPlain = exports.viaPlain = _mexpFn.default;
