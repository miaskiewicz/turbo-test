import * as fs from 'node:fs'
const rows = []
const row = (name, f) => { let r; try { r = ['value', JSON.stringify(f(), (k, v) => (v === undefined ? 'undef' : typeof v === 'function' ? `fn:${jest.isMockFunction(v) ? 'mock' : 'plain'}` : v))] } catch (e) { r = ['throws', `${e.name}: ${e.message}`] } rows.push([name, ...r]) }
test('jest.fn().whenCalledWith', () => {
  row('sub-mock', () => { const f = jest.fn(); const s = f.whenCalledWith(1); return [typeof f.whenCalledWith, jest.isMockFunction(s), s === f, typeof s.whenCalledWith] })
  row('match', () => { const f = jest.fn(); f.whenCalledWith(1, 'a').mockReturnValue('one'); return [f(1, 'a'), f(1), f(1, 'a', 2), f(2, 'a'), f.mock.calls.length] })
  row('deep / asymmetric', () => { const f = jest.fn(); f.whenCalledWith({ a: [1] }).mockReturnValue('obj'); f.whenCalledWith(expect.any(String)).mockReturnValue('str'); return [f({ a: [1] }), f({ a: [2] }), f('x'), f(3)] })
  row('fallback impl', () => { const f = jest.fn(() => 'base'); f.whenCalledWith(1).mockReturnValue('one'); return [f(1), f(2), f.getMockImplementation()()] })
  row('mockImplementation after', () => { const f = jest.fn(); f.whenCalledWith(1).mockReturnValue('one'); f.mockReturnValue('later'); return [f(1), f(2), f.getMockImplementation()()] })
  row('mockImplementation before', () => { const f = jest.fn().mockReturnValue('before'); f.whenCalledWith(1).mockReturnValue('one'); return [f(1), f(2)] })
  row('last persistent wins', () => { const f = jest.fn(); f.whenCalledWith(expect.anything()).mockReturnValue('any'); f.whenCalledWith(1).mockReturnValue('one'); return [f(1), f(2)] })
  row('earliest once first', () => { const f = jest.fn(); const a = f.whenCalledWith(expect.anything()); a.mockReturnValueOnce('any-once'); a.mockReturnValue('any'); f.whenCalledWith(1).mockReturnValueOnce('one-once'); return [f(1), f(1), f(1), f(1)] })
  row('parent once first', () => { const f = jest.fn(); f.whenCalledWith(1).mockReturnValue('one'); f.mockReturnValueOnce('parent-once'); return [f(1), f(1)] })
  row('empty sub-mock falls through', () => { const f = jest.fn(() => 'base'); f.whenCalledWith(1); return [f(1)] })
  row('sub-mock records', () => { const f = jest.fn(); const s = f.whenCalledWith(1).mockReturnValue('one'); f(1); f(2); return [s.mock.calls, f.mock.calls, s.mock.results.map((r) => r.value)] })
  row('this', () => { const f = jest.fn(); f.whenCalledWith(1).mockImplementation(function () { return this && this.k }); return [f.call({ k: 'ctx' }, 1)] })
  row('mockReset', () => { const f = jest.fn(); const s = f.whenCalledWith(1).mockReturnValue('one'); f.mockReset(); return [f(1), s(1), s.mock.calls.length] })
  row('mockReset then again', () => { const f = jest.fn(); f.whenCalledWith(1).mockReturnValue('one'); f.mockReset(); f.whenCalledWith(2).mockReturnValue('two'); return [f(1), f(2)] })
  row('spyOn', () => { const o = { m: (x) => 'real' + x }; const sp = jest.spyOn(o, 'm'); sp.whenCalledWith(1).mockReturnValue('one'); const r = [o.m(1), o.m(2)]; sp.mockRestore(); return [...r, o.m(3)] })
  row('withImplementation', () => { const f = jest.fn(); f.whenCalledWith(1).mockReturnValue('one'); const inside = []; f.withImplementation(() => 'w', () => { inside.push(f(1), f(2)) }); return [inside, f(1), f(2)] })
})
afterAll(() => fs.writeFileSync(process.env.OUT || 'when.jest.json', JSON.stringify(rows, null, 1)))
