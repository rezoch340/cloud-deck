import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('.', import.meta.url));
const buildRoot = process.env.CLOUD_DECK_BUILD_ROOT;

export default defineConfig({
  root,
  plugins: [tailwindcss(), react()],
  clearScreen: false,
  cacheDir: buildRoot ? path.join(buildRoot, 'vite-cache') : 'node_modules/.vite',
  build: { outDir: buildRoot ? path.join(buildRoot, 'dist') : 'dist', emptyOutDir: true },
  server: {
    host: '127.0.0.1',
    port: 18774,
    strictPort: true,
    fs: {
      allow: [
        root,
        path.resolve(root, '../manifest.json'),
        ...(buildRoot
          ? [path.join(buildRoot, 'node_modules'), path.join(buildRoot, 'vite-cache')]
          : []),
      ],
      deny: ['**/auth/**', '**/.git/**', '**/.claude/**', '**/.env*', '**/*.{pem,key,p12}'],
    },
    watch: { ignored: ['**/src-tauri/**'] },
  },
});
