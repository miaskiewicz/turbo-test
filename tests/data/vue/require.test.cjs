const { createApp, nextTick } = require('vue');
const component = require('./Counter.vue').default;
test('Vue SFCs load through the CommonJS require graph', async () => {
  const root = document.createElement('div'); document.body.appendChild(root);
  const app = createApp(component, { label: 'required' });
  try {
    app.mount(root); expect(root.textContent).toBe('required 1');
    root.querySelector('button').dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await nextTick();
    expect(root.textContent).toBe('required 2');
  }
  finally { app.unmount(); root.remove(); }
});
