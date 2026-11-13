const { createApp } = require('vue');
const component = require('./Counter.vue').default;
test('Vue SFCs load through the CommonJS require graph', () => {
  const root = document.createElement('div'); document.body.appendChild(root);
  const app = createApp(component, { label: 'required' });
  try { app.mount(root); expect(root.textContent).toBe('required 1'); }
  finally { app.unmount(); root.remove(); }
});
