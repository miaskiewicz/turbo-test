test('exceeds the timeout set by the Jest project', async () => {
  await new Promise(resolve => setTimeout(resolve, 100));
});
