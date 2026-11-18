// The terminals, as wbd last listed them, and what each prints as it comes.

import { createSignal } from 'solid-js';
import { ask, onEvent, status } from '../daemon';
import type { Terminal } from '../protocol/Terminal';

const [terminals, setTerminals] = createSignal<Terminal[]>([]);

export { terminals };

/** Who draws each terminal's output, by terminal id. */
const drawing = new Map<string, (data: string) => void>();

onEvent((event) => {
  if (event.event === 'terminals') setTerminals(event.list);
  else if (event.event === 'terminal') drawing.get(event.id)?.(event.data);
  // A new wbd has no terminals: the old ones went with the old wbd.
  else if (event.event === 'ready') setTerminals([]);
});

/** Hears what a terminal prints from now on. Returns how to stop. */
export function draw(id: string, onData: (data: string) => void) {
  drawing.set(id, onData);
  return () => drawing.delete(id);
}

/** The terminals that started in a folder or under it. */
export const terminalsIn = (dir: string) =>
  status() === 'ready' ? terminals().filter((t) => t.cwd === dir || t.cwd.startsWith(`${dir}/`)) : [];

export async function openTerminal(cwd: string) {
  return ask({ method: 'terminal.open', params: { cwd, cols: 80, rows: 12 } }, 'terminal');
}

/** The last of what a terminal printed, to draw it again. */
export async function printed(id: string) {
  return (await ask({ method: 'terminal.read', params: { id } }, 'printed')).text;
}

export const type = (id: string, data: string) =>
  void ask({ method: 'terminal.input', params: { id, data } }, 'done').catch(() => {});

export const resize = (id: string, cols: number, rows: number) =>
  void ask({ method: 'terminal.resize', params: { id, cols, rows } }, 'done').catch(() => {});

export const closeTerminal = (id: string) => ask({ method: 'terminal.close', params: { id } }, 'done');
