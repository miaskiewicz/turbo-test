module.exports = { test: (v) => v && v.tag === 'T', print: (v, serialize) => `Tagged(${serialize(v.body)})` }
