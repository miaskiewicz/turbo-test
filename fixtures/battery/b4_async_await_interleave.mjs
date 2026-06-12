export async function scenario() {
  const o = [];
  async function a() { o.push('a1'); await null; o.push('a2'); await new Promise((r) => setTimeout(r, 0)); o.push('a3'); }
  o.push('start');
  const pr = a();
  o.push('after-call');
  Promise.resolve().then(() => o.push('micro'));
  await pr;
  o.push('end');
  return o;
}

// dual entry: plain script under node (oracle), test under turbo-test
if (typeof describe === "undefined") {
  scenario().then((o) => console.log("ORDER:" + o.join(",")));
} else {
  it("ordering parity", async () => { console.log("ORDER:" + (await scenario()).join(",")); });
}
