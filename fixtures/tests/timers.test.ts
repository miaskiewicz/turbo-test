describe('fake timers', () => {
  it('advanceTimersByTime fires due timers', () => {
    vi.useFakeTimers();
    let x = 0;
    setTimeout(() => { x = 1; }, 100);
    expect(x).toBe(0);
    vi.advanceTimersByTime(99);
    expect(x).toBe(0);
    vi.advanceTimersByTime(1);
    expect(x).toBe(1);
    vi.useRealTimers();
  });
  it('runAllTimers runs in due order', () => {
    vi.useFakeTimers();
    const seq: string[] = [];
    setTimeout(() => seq.push('a'), 10);
    setTimeout(() => seq.push('b'), 5);
    setTimeout(() => seq.push('c'), 20);
    vi.runAllTimers();
    expect(seq).toEqual(['b', 'a', 'c']);
    vi.useRealTimers();
  });
  it('setSystemTime + Date.now', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2020-06-15T00:00:00Z'));
    expect(Date.now()).toBe(new Date('2020-06-15T00:00:00Z').getTime());
    vi.advanceTimersByTime(1000);
    expect(Date.now()).toBe(new Date('2020-06-15T00:00:01Z').getTime());
    vi.useRealTimers();
  });
  it('clearTimeout cancels', () => {
    vi.useFakeTimers();
    let hit = false;
    const id = setTimeout(() => { hit = true; }, 10);
    clearTimeout(id);
    vi.advanceTimersByTime(100);
    expect(hit).toBe(false);
    vi.useRealTimers();
  });
  it('getTimerCount', () => {
    vi.useFakeTimers();
    setTimeout(() => {}, 1);
    setTimeout(() => {}, 2);
    expect(vi.getTimerCount()).toBe(2);
    vi.runAllTimers();
    expect(vi.getTimerCount()).toBe(0);
    vi.useRealTimers();
  });
});
