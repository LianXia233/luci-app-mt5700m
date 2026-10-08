import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';

// Semi ships a prebuilt stylesheet at dist/css/semi.min.css, but since 2.60
// its package.json "exports" map only whitelists per-component paths under
// lib/es and lib/cjs — `dist/*` is not listed, so a bare
// `import '@douyinfe/semi-ui/dist/css/semi.min.css'` is rejected at resolve
// time even though the file is on disk. Aliasing the specifier to the real
// path (rather than a relative ../node_modules hop) keeps resolution inside
// Vite and survives the dependency tree being reshuffled.
const semiCss = fileURLToPath(
  new URL('./node_modules/@douyinfe/semi-ui/dist/css/semi.min.css', import.meta.url),
);

// The WebUI is served from the router's uhttpd at /5700/, so every emitted URL
// must keep that prefix. `base` is what makes the hashed asset links in
// index.html resolve; without it the SPA 404s on every static request.
export default defineConfig({
  base: '/5700/',
  plugins: [
    react(),
    {
      // `src/vite-env.d.ts` declares `__APP_VERSION__`, which the footer and
      // the System > Info page read. Inject it from package.json so there is a
      // single source of truth for the version string.
      name: 'inject-app-version',
      transformIndexHtml(html) {
        return html.replace(/__APP_VERSION__/g, process.env.APP_VERSION ?? '3.0.0');
      },
    },
  ],
  resolve: {
    alias: [
      { find: '@', replacement: fileURLToPath(new URL('./src', import.meta.url)) },
      { find: /^@douyinfe\/semi-ui\/dist\/css\/semi\.min\.css$/, replacement: semiCss },
    ],
  },
  define: {
    __APP_VERSION__: JSON.stringify(process.env.APP_VERSION ?? '3.0.0'),
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // The router is a low-power ARM Cortex-A53 with limited RAM; a single
    // bundle keeps peak memory down and avoids a waterfall of module requests
    // over the (often slow) Wi-Fi link back to the router itself.
    chunkSizeWarningLimit: 1600,
    rollupOptions: {
      output: {
        // Keep the entry chunk stable-named so index.html and the deployed
        // assets/ layout match what the router already expects.
        entryFileNames: 'assets/index-[hash].js',
        chunkFileNames: 'assets/index-[hash].js',
        assetFileNames: 'assets/index-[hash].[ext]',
        manualChunks: undefined,
      },
    },
  },
  server: {
    host: true,
    port: 5173,
  },
});
