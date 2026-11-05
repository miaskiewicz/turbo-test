import * as fs from 'node:fs'
const rows = []
const row = (name, f) => { let r; try { r = ['value', JSON.stringify(f(), (k, v) => (v === undefined ? 'undef' : typeof v === 'function' ? `fn:${jest.isMockFunction(v) ? 'mock' : 'plain'}` : v))] } catch (e) { r = ['throws', `${e.name}: ${e.message}`] } rows.push([name, ...r]) }
const mk = () => { const o = { m: (x) => 'real' + x }; return [o, jest.spyOn(o, 'm')] }
const shapes = () => {
  const a = jest.fn(() => 'base'); a('x')
  const b = jest.fn(); b.mockImplementation(() => 'impl'); b.mockReturnValueOnce('once')
  const c = jest.fn().mockReturnValue('rv'); c.mockImplementationOnce(() => 'io')
  const [o, s] = mk(); s.mockReturnValue('spied')
  const w = jest.fn(() => 'base'); w.whenCalledWith(1).mockReturnValue('a')
  return { a, b, c, o, s, w }
}
const probe = ({ a, b, c, o, s, w }) => [a(), a.mock.calls.length, b(), b(), c(), c(), o.m(1), s.mock.calls.length, jest.isMockFunction(o.m), w(1), w(2), typeof a.getMockImplementation()]
test('jest mock reset / clear / restore', () => {
  row('mockReset', () => { const x = shapes(); for (const k of 'abcsw') x[k].mockReset(); return probe(x) })
  row('resetAllMocks', () => { const x = shapes(); jest.resetAllMocks(); return probe(x) })
  row('mockClear', () => { const x = shapes(); for (const k of 'abcsw') x[k].mockClear(); return probe(x) })
  row('clearAllMocks', () => { const x = shapes(); jest.clearAllMocks(); return probe(x) })
  row('mockRestore', () => { const x = shapes(); for (const k of 'abcsw') x[k].mockRestore(); return probe(x) })
  row('restoreAllMocks', () => { const x = shapes(); jest.restoreAllMocks(); return probe(x) })
  row('reset then reuse', () => { const f = jest.fn(() => 'base'); f.mockReset(); f.mockReturnValue('new'); const [o, s] = mk(); s.mockReset(); s.mockReturnValue('n2'); return [f(), o.m(1)] })
  row('when after reset', () => { const f = jest.fn(() => 'base'); f.whenCalledWith(1).mockReturnValue('a'); jest.resetAllMocks(); f.whenCalledWith(2).mockReturnValue('b'); return [f(1), f(2)] })
})
afterAll(() => fs.writeFileSync(process.env.OUT || 'reset.jest.json', JSON.stringify(rows, null, 1)))
