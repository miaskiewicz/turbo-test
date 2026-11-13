import { createApp } from 'vue';
import component from './Counter.vue';
await Promise.resolve();
test('Vue SFCs load through the native ESM graph', () => {
  const root = document.createElement('div'); document.body.appendChild(root);
  const app = createApp(component, { label: 'esm' });
  try { app.mount(root); expect(root.textContent).toBe('esm 1'); }
  finally { app.unmount(); root.remove(); }
});
