it.only.each([['o']])('only %s', () => {});
describe.only.each([['ds']])('only suite %s', () => { it('inner', () => {}); });
it('not run', () => {});
