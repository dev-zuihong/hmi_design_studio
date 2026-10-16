import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

export default defineConfig({
  plugins: [vue()],
  server: { host: '127.0.0.1', port: 1420, strictPort: true, watch: { ignored: ['**/target/**', '**/.cache/**', '**/artifacts/**', '**/dist*/**', '**/test-results/**'] } },
  clearScreen: false,
  build: { target: 'es2022', outDir: 'dist' },
});
