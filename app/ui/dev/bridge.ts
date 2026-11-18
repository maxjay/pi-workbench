// Development only: lets the page run in a plain browser, outside the app.
// `npm run dev` starts wbd for each page and carries its lines over a
// WebSocket at /wbd, the way the app carries them over Tauri.
//
// It runs the same wbd the app does: ~/.local/bin/wbd, in WSL on Windows.
// WBD names another one (on Windows, a path inside WSL).

import { type ChildProcessWithoutNullStreams, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import type { Plugin } from 'vite';
import { WebSocketServer } from 'ws';

function start(): ChildProcessWithoutNullStreams {
  const path = process.env.WBD;
  if (process.platform !== 'win32') return spawn(path ?? 'wbd', []);
  // A login shell, so wbd gets the PATH your terminal has.
  const run = path ? ['exec "$0"', path] : ['exec "$HOME/.local/bin/wbd"'];
  return spawn('wsl.exe', ['-e', 'sh', '-lc', ...run]);
}

export function bridge(): Plugin {
  return {
    name: 'wbd-bridge',
    apply: 'serve',
    configureServer(server) {
      const sockets = new WebSocketServer({ noServer: true });
      server.httpServer?.on('upgrade', (req, socket, head) => {
        if (req.url !== '/wbd') return;
        sockets.handleUpgrade(req, socket, head, (ws) => {
          const wbd = start();
          let last = '';
          createInterface({ input: wbd.stdout }).on('line', (line) => ws.send(line));
          createInterface({ input: wbd.stderr }).on('line', (line) => {
            last = line;
            console.error(`wbd: ${line}`);
          });
          wbd.on('error', (err) => ws.close(1011, `couldn't start wbd: ${err.message}`.slice(0, 120)));
          wbd.on('exit', () => ws.close(1000, (last ? `wbd stopped: ${last}` : 'wbd stopped').slice(0, 120)));
          ws.on('message', (data) => wbd.stdin.write(`${data.toString()}\n`));
          ws.on('close', () => wbd.kill());
        });
      });
    },
  };
}
