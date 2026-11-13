import { createApp, nextTick } from 'vue';
import Counter from './Counter';
import Options from './Options.vue';
import TemplateOnly from './TemplateOnly.vue';
import Asset from './Asset.vue';
import logo from './logo.png';
import logo2 from './logo2.png';

function mounted(component, props) {
  const root = document.createElement('div');
  document.body.appendChild(root);
  const app = createApp(component, props);
  app.mount(root);
  return { root, cleanup() { app.unmount(); root.remove(); } };
}
test('Vue script setup TypeScript renders props and updates reactive event state', async () => {
  const view = mounted(Counter, { label: 'count' });
  try {
    const button = view.root.querySelector('button');
    expect(button.textContent).toBe('count 1');
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await nextTick();
    expect(button.textContent).toBe('count 2');
  } finally { view.cleanup(); }
});
test('Vue scoped styles attach component scope attributes and real CSS', () => {
  const view = mounted(Counter, { label: 'styled' });
  try {
    const button = view.root.querySelector('button');
    expect(Counter.__scopeId).toMatch(/^data-v-[a-f0-9]+$/);
    expect(button.hasAttribute(Counter.__scopeId)).toBe(true);
    expect(getComputedStyle(button).color).toBe('rgb(10, 20, 30)');
  } finally { view.cleanup(); }
});
test('Vue options TypeScript and template-only SFCs compile and render', () => {
  const options = mounted(Options);
  const template = mounted(TemplateOnly);
  try {
    expect(options.root.textContent).toBe('options work');
    expect(template.root.textContent).toBe('template works');
    expect(template.root.querySelector('strong').hasAttribute(TemplateOnly.__scopeId)).toBe(true);
    expect(template.root.querySelector('span').hasAttribute(TemplateOnly.__scopeId)).toBe(true);
  } finally { options.cleanup(); template.cleanup(); }
});

test('Vue template image and srcset assets use real module imports', () => {
  const view = mounted(Asset);
  try {
    const image = view.root.querySelector('img');
    expect(image.getAttribute('src')).toBe(logo);
    expect(image.getAttribute('srcset')).toContain(logo + ' 1x');
    expect(image.getAttribute('srcset')).toContain(logo2 + ' 2x');
  } finally { view.cleanup(); }
});
