import { defineConfig } from 'vite';
import path from 'node:path';
import vue from '@vitejs/plugin-vue';

export default defineConfig({
  root: path.resolve('ui'),
  base: './',
  plugins: [vue()],
  clearScreen: false,
  server: {
    strictPort: true,
  },
  build: {
    outDir: path.resolve('dist'),
    emptyOutDir: true,
  },
});
