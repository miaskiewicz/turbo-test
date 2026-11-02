import * as fs from 'node:fs'
const desc = (o, k) => { let p = o; while (p) { const d = Object.getOwnPropertyDescriptor(p, k); if (d) return d; p = Object.getPrototypeOf(p) } }
const names = (o) => { const s = new Set(); let p = o; while (p && p !== Object.prototype && p !== Function.prototype) { for (const k of Object.getOwnPropertyNames(p)) s.add(k); p = Object.getPrototypeOf(p) } return [...s].sort() }
const typed = (o) => names(o).map((k) => { const d = desc(o, k); return k + ':' + (d.get ? 'get' : typeof d.value) })
test('surface', () => {
  const obj = { m() { return 1 } }
  const out = { jest: typed(jest) }
  fs.writeFileSync(process.env.OUT || 'members.jest.json', JSON.stringify(out, null, 1))
})
