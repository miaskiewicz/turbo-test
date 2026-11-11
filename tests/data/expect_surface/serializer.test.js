import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
const rows = []
const show = (v) => { try { return v === undefined ? 'undefined' : JSON.stringify(v) } catch { return String(v) } }
const t = (name, f) => { let r; try { const v = f(); r = v === undefined ? 'passed' : ['value', show(v)] } catch (e) { r = [e && e.name, e && e.message] } rows.push(JSON.stringify([name, r])) }
class Money { constructor(n, cur) { this.n = n; this.cur = cur } }
class Box { constructor(inner) { this.inner = inner } }
class Old { constructor(v) { this.v = v } }
class Cfg {}
test('no serializer yet', () => {
  expect(new Money(1, 'EUR')).toMatchInlineSnapshot(`
    Money {
      "cur": "EUR",
      "n": 1,
    }
  `)
})
test('serializers', () => {
  expect.addSnapshotSerializer({ test: (v) => v instanceof Money, serialize: (v) => `${v.n} ${v.cur}` })
  expect.addSnapshotSerializer({
    test: (v) => v instanceof Box,
    serialize: (v, config, indentation, depth, refs, printer) => `Box<${printer(v.inner, config, indentation, depth, refs)}> d=${depth} r=${refs.length} i=${JSON.stringify(indentation)}`,
  })
  expect.addSnapshotSerializer({
    test: (v) => v instanceof Old,
    print: (v, serialize, indent, opts, colors) => `Old(${serialize(v.v)}) ${JSON.stringify(indent('x\ny'))} ${JSON.stringify(Object.keys(opts).sort())} ${JSON.stringify(opts.edgeSpacing)} ${JSON.stringify(Object.keys(colors).sort())} ${JSON.stringify(colors.prop)}`,
  })
  expect.addSnapshotSerializer({
    test: (v) => v instanceof Cfg,
    serialize: (v, c) => JSON.stringify(Object.keys(c).sort()) + '\n' + JSON.stringify([c.callToJSON, c.escapeRegex, c.escapeString, c.indent, c.maxDepth, c.maxWidth, c.min, c.printBasicPrototype, c.printFunctionName, c.spacingInner, c.spacingOuter, c.plugins.length, typeof c.compareKeys, c.colors && Object.keys(c.colors).sort(), c.colors && c.colors.prop, c.printShadowRoot]),
  })
  expect(new Money(5, 'USD')).toMatchInlineSnapshot(`5 USD`)
  expect({ price: new Money(2, 'PLN'), list: [new Money(3, 'GBP')] }).toMatchInlineSnapshot(`
    {
      "list": [
        3 GBP,
      ],
      "price": 2 PLN,
    }
  `)
  expect(new Box({ a: [1, new Money(4, 'CHF')] })).toMatchInlineSnapshot(`
    Box<{
      "a": [
        1,
        4 CHF,
      ],
    }> d=0 r=0 i=""
  `)
  expect({ deep: { box: new Box(new Box('s')) } }).toMatchInlineSnapshot(`
    {
      "deep": {
        "box": Box<Box<"s"> d=2 r=2 i="    "> d=2 r=2 i="    ",
      },
    }
  `)
  expect(new Old({ a: 1 })).toMatchInlineSnapshot(`
    Old({
      "a": 1,
    }) "  x\\n  y" ["edgeSpacing","min","spacing"] "\\n" ["comment","content","prop","tag","value"] {"close":"","open":""}
  `)
  expect(new Cfg()).toMatchInlineSnapshot(`
    ["_outputLengthPerDepth","callToJSON","colors","compareKeys","escapeRegex","escapeString","indent","maxDepth","maxOutputLength","maxWidth","min","plugins","printBasicPrototype","printFunctionName","printShadowRoot","spacingInner","spacingOuter"]
    [true,true,false,"  ",null,null,false,false,false,"\\n","\\n",11,"undefined",["comment","content","prop","tag","value"],{"close":"","open":""},true]
  `)
  expect({ m: new Money(6, 'JPY') }).toMatchSnapshot()
  expect([new Box(1)]).toMatchSnapshot('hint')
  expect(() => { throw new Money(7, 'SEK') }).toThrowErrorMatchingInlineSnapshot(`7 SEK`)
  expect.addSnapshotSerializer({ test: (v) => v instanceof Money, serialize: (v) => `latest ${v.n}` })
  expect(new Money(8, 'NOK')).toMatchInlineSnapshot(`latest 8`)
  expect.addSnapshotSerializer({ test: (v) => typeof v === 'string' && v.startsWith('ser:'), serialize: (v) => v.slice(4).toUpperCase() })
  expect('ser:abc').toMatchInlineSnapshot(`ABC`)
  expect({ s: 'ser:x', t: 'plain' }).toMatchInlineSnapshot(`
    {
      "s": X,
      "t": "plain",
    }
  `)
  t('toEqual unaffected', () => expect(new Money(1, 'A')).toEqual(new Money(2, 'A')))
  t('toBe unaffected', () => expect({ m: new Money(1, 'A') }).toBe(1))
  fs.writeFileSync('out.txt', rows.join('\n') + '\n')
})
test('persists to the next test', () => {
  expect(new Money(9, 'DKK')).toMatchInlineSnapshot(`latest 9`)
})
test('bad serializer', () => {
  expect.addSnapshotSerializer({ test: (v) => v === 'bad' })
  let r
  try { expect('bad').toMatchInlineSnapshot(`"bad"`) } catch (e) { r = [e.name, e.message] }
  fs.appendFileSync('out.txt', JSON.stringify(['bad serializer', r]) + '\n')
})
