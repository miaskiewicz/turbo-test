// Persistent `ts.transpileModule` worker. The Rust side (src/tsc_worker.rs) spawns one
// instance per (project root, typescript dir), then streams length-prefixed JSON frames
// over stdio. Frame format on BOTH directions: `<decimal-length>\n<body-bytes>`.
// Request body: {"file","moduleKind","src"}. Response body: {"code"} on success, {"error"}
// on failure. Each file goes through the same logic that the pre-0.8.6 inline `node -e`
// script used — type-only imports moved onto a namespace import (TS 1272 remedy), dynamic
// `import(...)` preserved for the CommonJS loader, inlined decorator helpers + metadata.
const ts = require(process.argv[1]);

const configCache = new Map();
function projectOptionsFor(file) {
  const dir = require('path').dirname(file);
  if (configCache.has(dir)) return configCache.get(dir);
  const configFile = ts.findConfigFile(dir, ts.sys.fileExists);
  const parsed = configFile && ts.getParsedCommandLineOfConfigFile(configFile, {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} });
  const value = (parsed && parsed.options) || null;
  configCache.set(dir, value);
  return value;
}

function nsTypeOnlyImports(file, isTsx, text) {
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, isTsx ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const binds = new Map();
  for (const st of sf.statements) {
    const ic = ts.isImportDeclaration(st) && st.importClause;
    if (!ic || ic.isTypeOnly) continue;
    const add = (local, imported, el) => binds.set(local.text, { st, el, imported, refs: [], ok: true });
    if (ic.name) add(ic.name, 'default', ic);
    if (ic.namedBindings && ts.isNamedImports(ic.namedBindings)) {
      for (const el of ic.namedBindings.elements) {
        const imp = el.propertyName || el.name;
        if (!el.isTypeOnly && ts.isIdentifier(imp)) add(el.name, imp.text, el);
      }
    }
  }
  if (!binds.size) return text;
  const declKinds = [ts.isVariableDeclaration, ts.isParameter, ts.isFunctionDeclaration, ts.isFunctionExpression,
    ts.isClassDeclaration, ts.isClassExpression, ts.isInterfaceDeclaration, ts.isTypeAliasDeclaration,
    ts.isEnumDeclaration, ts.isModuleDeclaration, ts.isTypeParameterDeclaration, ts.isBindingElement,
    ts.isImportEqualsDeclaration, ts.isNamespaceImport, ts.isImportSpecifier, ts.isImportClause];
  const typeUse = (id) => {
    for (let n = id, q = id.parent; q; n = q, q = q.parent) {
      if (ts.isTypeQueryNode(q) || ts.isComputedPropertyName(q)) return false;
      if (ts.isExpressionWithTypeArguments(q)) {
        const h = q.parent;
        return ts.isHeritageClause(h) && (h.token === ts.SyntaxKind.ImplementsKeyword || ts.isInterfaceDeclaration(h.parent));
      }
      if (ts.isTypeNode(q)) return true;
    }
    return false;
  };
  const visit = (node) => {
    const b = ts.isIdentifier(node) && binds.get(node.text);
    if (b) {
      const p = node.parent;
      if (p === b.el) { /* the import binding itself */ }
      else if (declKinds.some((k) => k(p)) && p.name === node) b.ok = false; // shadowed
      else if (ts.isExportSpecifier(p)) b.ok = false; // re-exported: needs the binding
      else if ((ts.isPropertyAccessExpression(p) && p.name === node) || (ts.isQualifiedName(p) && p.right === node)
        || (p.propertyName === node) || (p.name === node && !ts.isShorthandPropertyAssignment(p))
        || ts.isLabeledStatement(p) || ts.isBreakOrContinueStatement(p)) { /* a member name, not a reference */ }
      else if (typeUse(node)) b.refs.push(node);
      else b.ok = false; // a value reference
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  const edits = [];
  const perDecl = new Map();
  for (const b of binds.values()) {
    if (!b.ok || !b.refs.length) continue;
    if (!perDecl.has(b.st)) perDecl.set(b.st, { ns: '__tt_ns' + perDecl.size, moved: new Set() });
    const d = perDecl.get(b.st);
    d.moved.add(b.el);
    for (const r of b.refs) edits.push([r.getStart(sf), r.end, d.ns + '.' + b.imported]);
  }
  for (const [st, d] of perDecl) {
    const ic = st.importClause;
    const nb = ic.namedBindings;
    const keepDefault = ic.name && !d.moved.has(ic);
    const keepNamed = nb && ts.isNamedImports(nb) ? nb.elements.filter((e) => !d.moved.has(e)) : [];
    const from = st.moduleSpecifier.getText(sf);
    const parts = [];
    if (keepDefault) parts.push(ic.name.text);
    if (nb && ts.isNamespaceImport(nb)) parts.push(nb.getText(sf));
    if (keepNamed.length) parts.push('{ ' + keepNamed.map((e) => e.getText(sf)).join(', ') + ' }');
    const clause = parts.length ? 'import ' + parts.join(', ') + ' from ' + from + '; ' : '';
    edits.push([st.getStart(sf), st.end, clause + 'import * as ' + d.ns + ' from ' + from + ';']);
  }
  edits.sort((a, b) => b[0] - a[0]);
  let out = text;
  for (const [s, e, r] of edits) out = out.slice(0, s) + r + out.slice(e);
  return out;
}

function transformOne(file, moduleKind, src) {
  const isTsx = file.endsWith('.tsx');
  const projectOptions = projectOptionsFor(file);
  const commonjsNoInterop = projectOptions && projectOptions.module === ts.ModuleKind.CommonJS && projectOptions.esModuleInterop === false;
  let input = src;
  try { input = nsTypeOnlyImports(file, isTsx, src); } catch (e) {} // e.g. a typescript without the parser API
  let dynamicName = '__ttDynamicNamespace';
  while (input.includes(dynamicName)) dynamicName += '_';
  let usesDynamicNamespace = false;
  function preserveDynamicNamespaces(context) {
    const visit = (node) => {
      if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        usesDynamicNamespace = true;
        return ts.factory.createCallExpression(ts.factory.createIdentifier(dynamicName), undefined, node.arguments.map(arg => ts.visitNode(arg, visit)));
      }
      return ts.visitEachChild(node, visit, context);
    };
    return source => ts.visitNode(source, visit);
  }
  // Follow the project's tsconfig `target` + `useDefineForClassFields` as the fallback path
  // does (turbo-test#438). Defaults: target ES2021, useDefineForClassFields true iff target
  // >= ES2022 (TypeScript's own rule).
  const projectTarget = projectOptions && projectOptions.target;
  const effectiveTarget = projectTarget !== undefined && projectTarget !== null ? projectTarget : ts.ScriptTarget.ES2021;
  const useDefineForClassFields = projectOptions && projectOptions.useDefineForClassFields !== undefined
    ? projectOptions.useDefineForClassFields
    : effectiveTarget >= ts.ScriptTarget.ES2022;
  const out = ts.transpileModule(input, {
    transformers: { before: moduleKind === 'commonjs' && commonjsNoInterop ? [preserveDynamicNamespaces] : [] },
    fileName: file,
    compilerOptions: {
      module: moduleKind === 'commonjs' ? ts.ModuleKind.CommonJS : ts.ModuleKind.ESNext,
      target: effectiveTarget,
      experimentalDecorators: true,
      emitDecoratorMetadata: true,
      esModuleInterop: !(moduleKind === 'commonjs' && commonjsNoInterop),
      importHelpers: false,
      isolatedModules: true,
      jsx: isTsx ? ts.JsxEmit.ReactJSX : undefined,
      useDefineForClassFields,
    },
  });
  const dynamicHelper = usesDynamicNamespace ? '\nfunction ' + dynamicName + '(spec) { return Promise.resolve().then(() => globalThis.__ttToESM(require(spec))); }\n' : '';
  return out.outputText + dynamicHelper;
}

// Length-prefixed JSON request loop. One concurrent request (module loading is single
// threaded on the Rust side), so no ordering tag — we answer in input order.
let inbuf = Buffer.alloc(0);
function nextFrame() {
  const nl = inbuf.indexOf(0x0a);
  if (nl < 0) return null;
  const n = parseInt(inbuf.slice(0, nl).toString('ascii').trim(), 10);
  if (!(n >= 0) || inbuf.length < nl + 1 + n) return null;
  const body = inbuf.slice(nl + 1, nl + 1 + n);
  inbuf = inbuf.slice(nl + 1 + n);
  return body;
}
function answer(body) {
  let resp;
  try {
    const req = JSON.parse(body.toString('utf8'));
    resp = { code: transformOne(req.file, req.moduleKind, req.src) };
  } catch (e) {
    resp = { error: String((e && e.stack) || e) };
  }
  const out = Buffer.from(JSON.stringify(resp));
  process.stdout.write(String(out.length) + '\n');
  process.stdout.write(out);
}
function dispatch() {
  let body;
  while ((body = nextFrame()) !== null) answer(body);
}
process.stdin.on('data', (chunk) => { inbuf = Buffer.concat([inbuf, chunk]); dispatch(); });
process.stdin.on('end', () => process.exit(0));
