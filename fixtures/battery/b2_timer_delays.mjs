export async function scenario() {
  const o = [];
  setTimeout(() => o.push('t10'), 10);
  setTimeout(() => o.push('t0a'), 0);
  setTimeout(() => o.push('t0b'), 0);
  setTimeout(() => o.push('t5'), 5);
  await new Promise((r) => setTimeout(r, 20));
  return o;
}

// dual entry: plain script under node (oracle), test under turbo-test
if (typeof describe === "undefined") {
  scenario().then((o) => console.log("ORDER:" + o.join(",")));
} else {
  it("ordering parity", async () => { console.log("ORDER:" + (await scenario()).join(",")); });
}
