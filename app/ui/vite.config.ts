import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { bridge } from './dev/bridge';

export default defineConfig({
  plugins: [solid(), bridge()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // Nothing is inlined as data: the app's content security policy only lets files from itself load.
  build: { target: 'es2022', assetsInlineLimit: 0 },
});
