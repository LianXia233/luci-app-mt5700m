import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';
import { readFileSync } from 'node:fs';

// 应用版本单一来源：package.json。构建时注入 __APP_VERSION__，
// 避免 header/footer 里出现硬编码版本号与实际发布版本不一致。
const pkg = JSON.parse(readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf-8'));

export default defineConfig({
  base: '/5700/',
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version || '0.0.0'),
  },
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    port: 5173,
    host: true,
    proxy: {
      '/cgi-bin': {
        target: 'http://192.168.1.1',
        changeOrigin: true,
      },
    },
  },
  preview: {
    port: 4173,
  },
});
