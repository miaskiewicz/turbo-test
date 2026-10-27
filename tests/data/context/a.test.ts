import { afterEach, beforeEach, describe, expect, it } from 'vitest';
const log = (...a: unknown[]) => console.log(...a);
it('noted', (ctx) => { ctx.skip('not today'); });
it('named', (ctx) => { expect(ctx.task.name).toBe('named'); });
it('hooks', ({ onTestFinished }) => { onTestFinished(() => console.log('finished')); });
describe('d', () => {
  beforeEach((ctx) => { ctx.fromBefore = 7; });
  afterEach((ctx) => { log('ae', ctx.task.name, ctx.task.result.state); });
  it('task', (ctx) => {
    log(Object.keys(ctx).sort().join(','), typeof ctx, ctx.fromBefore);
    log(Object.keys(ctx.task).sort().join(','));
    const t = ctx.task;
    log(t.type, t.mode, t.id, t.fullName, t.fullTestName, t.retry, t.timeout, t.meta, t.fails, t.each);
    log(t.suite.type, t.suite.name, t.suite.id, t.suite.fullName, t.suite.mode, t.suite.suite, t.file.type, t.file.name, t.file.id, t.file.mode);
    log(t.result.state, t.result.retryCount, t.result.repeatCount, t.annotations, t.artifacts);
    log(ctx.signal instanceof AbortSignal, ctx.signal.aborted, ctx.expect === expect, typeof ctx.expect);
    try { ctx(); } catch (e) { log(e.message); }
  });
  it('order', ({ onTestFinished, onTestFailed }) => {
    onTestFinished((c) => log('fin1', c.task.name, c.task.result.state));
    onTestFinished(() => log('fin2'));
    onTestFailed(() => log('never'));
  });
  it('fails', ({ onTestFailed, onTestFinished }) => {
    onTestFailed((c) => log('onFailed', c.task.result.state, c.task.result.errors.length, c.task.result.errors[0].message));
    onTestFinished(() => log('fin3'));
    throw new Error('boom');
  });
  it('finthrow', ({ onTestFinished }) => { onTestFinished(() => { throw new Error('in finished'); }); });
  it('nested', ({ onTestFinished }) => { onTestFinished((c) => { try { c.onTestFinished(() => {}); } catch (e) { log(e.message); } }); });
  it('skipcond', (ctx) => { ctx.skip(false, 'no'); log('after skip(false)'); ctx.skip(true, 'cond note'); log('unreached'); });
  it('skipcatch', (ctx) => { try { ctx.skip('caught'); } catch (e) { log('caught', e.name, e.message, e.code); } });
  it('ctxexpect', (ctx) => { ctx.expect.assertions(1); ctx.expect(1).toBe(1); });
  it('retried', { retry: 2 }, (ctx) => { log('try', ctx.task.result.retryCount, ctx.task.retry); if (ctx.task.result.retryCount < 1) throw new Error('first'); });
});
describe('bs', () => {
  beforeEach((ctx) => { ctx.skip('from beforeEach'); });
  it('skipped by hook', () => { log('unreached'); });
});
it('top', (ctx) => { log(ctx.task.suite, ctx.task.file.name, ctx.task.id); });
it.each([[1, 2]])('each %i', (a, b, c) => { log('each', a, b, typeof c); });
it.for([[1, 2]])('for %i', (row, ctx) => { log('for', row, ctx.task.name, typeof ctx.skip); });
const ext = it.extend({ val: async ({ task }, use) => { await use(task.name); } });
ext('fixture', ({ val, task, skip }) => { log('fixture', val, task.name, typeof skip); });
it('annotated', async (ctx) => { const a = await ctx.annotate('note one'); log(a.message, a.type, a.location.line, a.location.column, a.location.file.endsWith('/a.test.ts')); await ctx.annotate('warn <two>', 'warning'); });
