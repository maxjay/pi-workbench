// The workbench's terminal tool. A codespace's terminals are shells the
// user types in too; this lets Pi list them, read what they show, run a
// command in one, or open one. It asks wb-agent over the socket in
// WB_TERMINALS, one line each way.

import { connect } from 'node:net';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';
import { Type } from 'typebox';

const Params = Type.Object({
  action: Type.Union([Type.Literal('list'), Type.Literal('read'), Type.Literal('run'), Type.Literal('open')], {
    description: 'list the terminals, read one, run a command in one, or open a new one',
  }),
  id: Type.Optional(Type.String({ description: 'The terminal, as list names it (read and run)' })),
  command: Type.Optional(Type.String({ description: 'The command to type and run (run)' })),
  lines: Type.Optional(Type.Number({ description: 'How many of the last lines to read (read; default 60)' })),
  wait: Type.Optional(
    Type.Number({ description: 'Seconds to wait for the command to finish before returning (run; default 20)' }),
  ),
});

const say = (text: string) => ({ content: [{ type: 'text' as const, text }], details: {} });

function ask(path: string, request: Record<string, unknown>, signal?: AbortSignal) {
  return new Promise<{ text?: string; error?: string }>((resolve) => {
    const socket = connect(path);
    let buffer = '';
    let done = false;
    const finish = (result: { text?: string; error?: string }) => {
      if (done) return;
      done = true;
      signal?.removeEventListener('abort', stop);
      socket.destroy();
      resolve(result);
    };
    const stop = () => finish({ error: 'The run was stopped.' });
    signal?.addEventListener('abort', stop);
    socket.setEncoding('utf8');
    socket.on('connect', () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on('data', (chunk: string) => {
      buffer += chunk;
      const end = buffer.indexOf('\n');
      if (end < 0) return;
      try {
        finish(JSON.parse(buffer.slice(0, end)));
      } catch {
        finish({ error: 'The workbench sent a reply this tool could not read.' });
      }
    });
    socket.on('error', (err) => finish({ error: `Could not reach the workbench: ${err.message}` }));
    socket.on('close', () => finish({ error: 'The workbench closed the connection without a reply.' }));
  });
}

export default function terminal(pi: ExtensionAPI) {
  pi.registerTool({
    name: 'terminal',
    label: 'Terminal',
    description:
      "Use the codespace's terminals, which the user sees and types in too. List them to see what runs " +
      'where; read one to see its output, such as a server log or a failed build; run a command in one ' +
      'that is at its prompt, to leave it there for the user to see; or open a new one. Prefer bash for ' +
      'commands whose output only you need.',
    parameters: Params,

    async execute(_toolCallId, params, signal) {
      const path = process.env.WB_TERMINALS;
      if (!path) return say('Error: this session is not running in a workbench codespace, so it has no terminals.');
      const reply = await ask(path, { ...params, cwd: process.cwd() }, signal);
      return say(reply.error ? `Error: ${reply.error}` : (reply.text ?? ''));
    },
  });
}
