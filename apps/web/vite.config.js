import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
export default defineConfig({
  base: '/workspace/',
  plugins: [svelte()],
  build: { outDir: 'build', sourcemap: false },
});
