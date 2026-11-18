const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const mongoose = require(process.env.TURBO_MONGOOSE_ORACLE || 'mongoose');
testCase('real Mongoose accepts the active Date constructor and casts effectiveFrom', () => {
  const schema = new mongoose.Schema({ effectiveFrom: {type: Date, required: true}, history: [Date] });
  assert.equal(schema.path('effectiveFrom').instance, 'Date');
  const Model = mongoose.model('DateFacadeCompatibility', schema);
  try {
    const document = new Model({effectiveFrom:'2024-03-01',history:['2024-03-02']});
    assert.equal(document.validateSync(), undefined);
    assert.equal(document.effectiveFrom.toISOString(), '2024-03-01T00:00:00.000Z');
    assert.equal(document.history[0].toISOString(), '2024-03-02T00:00:00.000Z');
  } finally { mongoose.deleteModel('DateFacadeCompatibility'); }
});
