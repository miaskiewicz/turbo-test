import * as fs from 'node:fs'
const rows = []
const row = (name, f) => { let r; try { r = ['value', JSON.stringify(f(), (k, v) => (typeof v === 'function' ? `fn:${jest.isMockFunction(v) ? 'mock' : 'plain'}` : v))] } catch (e) { r = ['throws', `${e.name}: ${e.message}`] } rows.push([name, ...r]) }
test('jest module members', () => {
  row('createMockFromModule', () => { const m = jest.createMockFromModule('./helper.cjs'); return [m.add, m.add(1, 2), m.n, m.list, new m.C().m(), jest.isMockFunction(m.C)] })
  row('setMock', () => { const r = jest.setMock('./helper.cjs', { x: 1 }); return [r === jest, jest.requireMock('./helper.cjs')] })
  row('deepUnmock', () => jest.deepUnmock('./helper.cjs') === jest)
  row('unstable_mockModule', () => jest.unstable_mockModule('./helper.cjs', () => ({})) === jest)
  row('unstable_unmockModule', () => jest.unstable_unmockModule('./helper.cjs') === jest)
  row('getSeed', () => { const s = jest.getSeed(); return [Number.isInteger(s), s >= -0x80000000 && s <= 0x7fffffff, s === jest.getSeed()] })
  row('isEnvironmentTornDown', () => jest.isEnvironmentTornDown())
  row('mock chain', () => [jest.clearAllMocks(), jest.resetAllMocks(), jest.restoreAllMocks()].map((x) => x === jest))
})
afterAll(() => fs.writeFileSync(process.env.OUT || 'modules.jest.json', JSON.stringify(rows, null, 1)))
