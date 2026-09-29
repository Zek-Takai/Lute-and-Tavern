import { defineConfig } from 'vite';

// Tauri serves the built files; the dev server runs on a fixed port the app loads in development.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'safari16', outDir: 'dist', chunkSizeWarningLimit: 1200 },
});
