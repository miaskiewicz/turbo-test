import { defineConfig } from 'vitest/config';
import vue from '@vitejs/plugin-vue';
export default defineConfig({ plugins: [vue()], resolve: { extensions: ['.vue', '.mjs', '.js', '.ts', '.jsx', '.tsx', '.json'] }, test: { environment: 'jsdom', globals: true, include: ['vue.test.js', 'esm.test.mjs'], css: { include: [/.+/] } } });
