// Web Streams must actually stream (issue #15). The old runtime stub exposed the globals but its
// reader always resolved `{ done: true }`, so every consumer silently saw an empty stream.
import { describe, it, expect } from 'vitest';

async function readAll(stream: ReadableStream<string>): Promise<string> {
  const reader = stream.getReader();
  let out = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    out += value;
  }
  return out;
}

describe('ReadableStream', () => {
  it('is a function on the global', () => {
    expect(typeof ReadableStream).toBe('function');
  });

  it('delivers chunks enqueued from start(), then reports done', async () => {
    const stream = new ReadableStream<string>({
      start(controller) {
        controller.enqueue('a');
        controller.enqueue('b');
        controller.close();
      },
    });
    expect(await readAll(stream)).toBe('ab');
  });

  it('delivers chunks pulled lazily', async () => {
    let n = 0;
    const stream = new ReadableStream<string>({
      pull(controller) {
        if (n === 3) return controller.close();
        controller.enqueue(String(n++));
      },
    });
    expect(await readAll(stream)).toBe('012');
  });

  it('supports an async start()', async () => {
    const stream = new ReadableStream<string>({
      async start(controller) {
        await Promise.resolve();
        controller.enqueue('x');
        controller.close();
      },
    });
    expect(await readAll(stream)).toBe('x');
  });

  it('is async-iterable', async () => {
    const stream = new ReadableStream<number>({
      start(c) { c.enqueue(1); c.enqueue(2); c.close(); },
    });
    const seen: number[] = [];
    for await (const chunk of stream as any) seen.push(chunk);
    expect(seen).toEqual([1, 2]);
  });

  it('locks while a reader is held and reports locked', () => {
    const stream = new ReadableStream({ start(c) { c.close(); } });
    expect(stream.locked).toBe(false);
    stream.getReader();
    expect(stream.locked).toBe(true);
    expect(() => stream.getReader()).toThrow();
  });

  it('propagates controller.error() to the pending read', async () => {
    const boom = new Error('boom');
    const stream = new ReadableStream({ start(c) { c.error(boom); } });
    await expect(stream.getReader().read()).rejects.toThrow('boom');
  });

  it('cancel() runs the source cancel and ends the stream', async () => {
    let reason: unknown;
    const stream = new ReadableStream<string>({
      start(c) { c.enqueue('a'); },
      cancel(r) { reason = r; },
    });
    await stream.cancel('done here');
    expect(reason).toBe('done here');
    expect(await readAll(stream)).toBe('');
  });

  it('tee() feeds both branches the same chunks', async () => {
    const stream = new ReadableStream<string>({
      start(c) { c.enqueue('a'); c.enqueue('b'); c.close(); },
    });
    const [left, right] = stream.tee();
    expect(await readAll(left)).toBe('ab');
    expect(await readAll(right)).toBe('ab');
  });
});

describe('WritableStream', () => {
  it('receives written chunks in order and closes', async () => {
    const seen: string[] = [];
    let closed = false;
    const ws = new WritableStream<string>({
      write(chunk) { seen.push(chunk); },
      close() { closed = true; },
    });
    const writer = ws.getWriter();
    await writer.write('a');
    await writer.write('b');
    await writer.close();
    expect(seen).toEqual(['a', 'b']);
    expect(closed).toBe(true);
  });

  it('abort() reaches the sink', async () => {
    let reason: unknown;
    const ws = new WritableStream({ abort(r) { reason = r; } });
    const writer = ws.getWriter();
    await writer.abort('nope');
    expect(reason).toBe('nope');
  });
});

describe('TransformStream', () => {
  it('transforms chunks between its writable and readable sides', async () => {
    const upper = new TransformStream<string, string>({
      transform(chunk, controller) { controller.enqueue(chunk.toUpperCase()); },
      flush(controller) { controller.enqueue('!'); },
    });
    const writer = upper.writable.getWriter();
    const read = readAll(upper.readable);
    await writer.write('a');
    await writer.write('b');
    await writer.close();
    expect(await read).toBe('AB!');
  });

  it('is subclassable (libs do `class X extends TransformStream` at module load)', async () => {
    class Doubler extends TransformStream<number, number> {
      constructor() {
        super({ transform(chunk, controller) { controller.enqueue(chunk * 2); } });
      }
    }
    const d = new Doubler();
    const writer = d.writable.getWriter();
    const reader = d.readable.getReader();
    await writer.write(21);
    expect((await reader.read()).value).toBe(42);
  });
});

describe('pipeTo / pipeThrough', () => {
  it('pipes a readable into a writable', async () => {
    const seen: string[] = [];
    const rs = new ReadableStream<string>({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close(); } });
    const ws = new WritableStream<string>({ write(chunk) { seen.push(chunk); } });
    await rs.pipeTo(ws);
    expect(seen).toEqual(['a', 'b']);
  });

  it('pipeThrough returns the transform readable with transformed chunks', async () => {
    const rs = new ReadableStream<string>({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close(); } });
    const out = rs.pipeThrough(
      new TransformStream<string, string>({
        transform(chunk, controller) { controller.enqueue(chunk.toUpperCase()); },
      }),
    );
    expect(await readAll(out)).toBe('AB');
  });
});
