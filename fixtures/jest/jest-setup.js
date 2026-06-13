// jest setupFiles run before the test framework — set a marker the spec asserts (proves the jest
// config's setupFiles array was read + executed, with <rootDir> resolved).
process.env.JEST_SETUP_RAN = 'yes';

// Minimal reflect-metadata for the decorator-metadata fixture (a real jest project imports the
// `reflect-metadata` package; the fixture provides just enough here).
(function () {
  const store = new WeakMap();
  const k = (mk, p) => String(mk) + ' ' + String(p);
  const slot = (t) => { let m = store.get(t); if (!m) { m = new Map(); store.set(t, m); } return m; };
  Reflect.defineMetadata = (mk, mv, t, p) => slot(t).set(k(mk, p), mv);
  Reflect.getMetadata = (mk, t, p) => { const m = store.get(t); return m ? m.get(k(mk, p)) : undefined; };
  Reflect.metadata = (mk, mv) => (t, p) => Reflect.defineMetadata(mk, mv, t, p);
})();
