export async function scenario() {
  const o = [];
  process.nextTick(() => { o.push('nt1'); process.nextTick(() => o.push('nt2')); });
  Promise.resolve().then(() => { o.push('p1'); return Promise.resolve(); }).then(() => o.push('p2'));
  await new Promise((r) => setTimeout(r, 1));
  return o;
}

// dual entry: plain script under node (oracle), test under turbo-test
if (typeof describe === "undefined") {
  scenario().then((o) => console.log("ORDER:" + o.join(",")));
} else {
  it("ordering parity", async () => { console.log("ORDER:" + (await scenario()).join(",")); });
}
