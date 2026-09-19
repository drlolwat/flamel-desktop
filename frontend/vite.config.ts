import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

const BACKEND = process.env.VITE_BACKEND_URL ?? 'http://localhost:8181';
const OUT_DIR = process.env.VITE_OUT_DIR ?? 'dist';
// When set, the app auto-connects to this server URL on load (dev mode only)
const DEV_SERVER = process.env.FLAMEL_DEV_SERVER ?? '';

export default defineConfig(({ mode }) => {
  const appMode = process.env.VITE_MODE ?? (mode === 'desktop' ? 'local' : mode === 'web' ? 'web' : '');
  const documentTitle = appMode === 'web' ? 'Flamel Web' : 'flamel';
  return {
    define: {
      'import.meta.env.VITE_MODE': JSON.stringify(appMode),
      'import.meta.env.VITE_DEV_SERVER': JSON.stringify(DEV_SERVER),
    },
    build: { outDir: OUT_DIR, cssMinify: 'lightningcss' },
    plugins: [
      {
        name: 'flamel-document-title',
        transformIndexHtml(html) {
          return html.replace('<title>flamel</title>', `<title>${documentTitle}</title>`);
        },
      },
      react(),
      tailwindcss(),
    ],
    server: {
      port: 3030,
      host: '127.0.0.1',
      
      proxy: {
        '/api': {
          target: BACKEND,
          changeOrigin: true,
          // selfHandleResponse bypasses http-proxy's internal response buffering entirely.
          // We manually pipe the response so NDJSON chunks arrive immediately.
          selfHandleResponse: true,
          configure: (proxy) => {
            proxy.on('proxyRes', (proxyRes, _req, res: any) => {
              // Forward status + headers directly (drop content-length to allow streaming)
              const headers = { ...proxyRes.headers };
              delete headers['content-length'];
              delete headers['content-encoding'];
              res.writeHead(proxyRes.statusCode || 200, headers);
              // Pipe body directly — no buffering
              proxyRes.pipe(res);
            });
            proxy.on('error', (_err, _req, res: any) => {
              if (res && !res.headersSent) {
                res.writeHead(502);
                res.end('Bad Gateway');
              }
            });
          },
        },
        '/login': { target: BACKEND, changeOrigin: true },
        '/logout': { target: BACKEND, changeOrigin: true },
        '/share/': { target: BACKEND, changeOrigin: true },
      },
    },
  };
});
