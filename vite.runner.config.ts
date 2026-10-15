import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { resolve } from 'node:path';

export default defineConfig({
  plugins: [vue({ template: { compilerOptions: { isCustomElement: (tag) => tag.startsWith('fluent-') } } })],
  build: { target: 'es2022', outDir: 'dist-runner', rollupOptions: { input: resolve(import.meta.dirname, 'runner.html') } },
});
