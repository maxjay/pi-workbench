// Development only: lets the UI run in a plain browser, outside the app.
// `npm run dev` starts wb-agent for each page and carries its lines over a
// WebSocket at /agent, the way the app carries them over Tauri.
//
// WB_AGENT picks the command; by default it's the debug build.

import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import type { Plugin } from 'vite';
import { WebSocketServer } from 'ws';

export function agentBridge(): Plugin {
  return {
    name: 'agent-bridge',
    apply: 'serve',
    configureServer(server) {
      const sockets = new WebSocketServer({ noServer: true });
      server.httpServer?.on('upgrade', (req, socket, head) => {
        if (req.url !== '/agent') return;
        sockets.handleUpgrade(req, socket, head, (ws) => {
          const command = process.env.WB_AGENT ?? resolve(__dirname, '../../../target/debug/wb-agent');
          const agent = spawn(command, [], { stdio: ['pipe', 'pipe', 'inherit'] });
          createInterface({ input: agent.stdout }).on('line', (line) => ws.send(line));
          agent.on('exit', (code) => ws.close(1000, `wb-agent exited (${code ?? 'signal'})`));
          ws.on('message', (data) => agent.stdin.write(`${data.toString()}\n`));
          ws.on('close', () => agent.kill());
        });
      });
    },
  };
}
