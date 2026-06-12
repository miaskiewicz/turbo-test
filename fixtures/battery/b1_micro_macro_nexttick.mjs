export async function scenario() {
  const o = [];
  o.push('sync-start');
  setTimeout(() => o.push('timeout0'), 0);
  Promise.resolve().then(() => o.push('promise1'));
  process.nextTick(() => o.push('nextTick'));
  o.push('sync-end');
  await new Promise((r) => setTimeout(r, 5));
  o.push('after-await');
  return o;
}

// dual entry: plain script under node (oracle), test under turbo-test
if (typeof describe === "undefined") {
  scenario().then((o) => console.log("ORDER:" + o.join(",")));
} else {
  it("ordering parity", async () => { console.log("ORDER:" + (await scenario()).join(",")); });
}
