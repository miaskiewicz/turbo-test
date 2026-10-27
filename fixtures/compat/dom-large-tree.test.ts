import { describe, expect, it } from 'vitest';

// A render that touches thousands of nodes: ownership bookkeeping used to run after every
// mutating call and cost O(live wrappers) each time, so building n nodes was O(n^2) (a few-thousand
// node React tree took minutes). The contract this pins: it finishes quickly, and a wrapper that
// only the tree references still keeps its identity and expandos across GC while it is attached.
describe('large DOM trees', () => {
  it('builds and mutates 6000 nodes within a bounded time', () => {
    const started = Date.now();
    const host = document.createElement('div');
    document.body.appendChild(host);
    for (let i = 0; i < 2000; i++) {
      const row = document.createElement('div');
      row.setAttribute('data-i', String(i));
      const a = document.createElement('span');
      a.textContent = `a${i}`;
      const b = document.createElement('span');
      b.className = 'b';
      row.append(a, b);
      host.appendChild(row);
    }
    expect(host.children.length).toBe(2000);
    for (let i = 1998; i >= 0; i -= 2) host.children[i].remove(); // back to front: indexes shift
    expect(host.children.length).toBe(1000);
    expect(Date.now() - started).toBeLessThan(20_000);
  });

  it('keeps expandos on attached nodes nothing else references', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    for (let i = 0; i < 1500; i++) {
      const el = document.createElement('p');
      (el as any).marker = i;
      host.appendChild(el);
    }
    // Churn the heap so scavenges and mark-compacts run while only the tree holds the wrappers.
    let sink: unknown[] = [];
    for (let round = 0; round < 40; round++) {
      sink = Array.from({ length: 20_000 }, (_, k) => ({ k, round, s: `x${k}` }));
    }
    expect(sink.length).toBe(20_000);
    for (let i = 0; i < 1500; i += 97) expect((host.children[i] as any).marker).toBe(i);
  });
});
