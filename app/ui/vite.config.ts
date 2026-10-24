import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { agentBridge } from './dev/agent-bridge';

export default defineConfig({
  plugins: [solid(), agentBridge()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022' },
});
