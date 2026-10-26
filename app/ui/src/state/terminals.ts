// The terminals wb-agent runs, and what they print. Each terminal's view
// listens for its own output; what arrives before it does waits for it.

import { createEffect, createRoot } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { ask, onEvent, status } from '../agent';
import type { Terminal } from '../protocol/Terminal';

const [terminals, setTerminals] = createStore<Record<string, Terminal>>({});

export { terminals };

/** The terminals in a folder or under it, oldest first. */
export function terminalsIn(dir: string): Terminal[] {
  return Object.values(terminals)
    .filter((t) => t.cwd === dir || t.cwd.startsWith(`${dir}/`))
    .sort((a, b) => Number(a.id.slice(1)) - Number(b.id.slice(1)));
}

const writers = new Map<string, (data: string) => void>();
const early = new Map<string, string>();

onEvent((event) => {
  if (event.event === 'terminals') {
    setTerminals(reconcile(Object.fromEntries(event.list.map((t) => [t.id, t]))));
  } else if (event.event === 'terminal') {
    const write = writers.get(event.id);
    if (write) write(event.data);
    else early.set(event.id, (early.get(event.id) ?? '') + event.data);
  }
});

createRoot(() =>
  createEffect(() => {
    if (status() === 'down') setTerminals(reconcile({}));
  }),
);

/**
 * Has `write` hear what a terminal prints from now on, after what it has
 * printed so far. Returns a function that stops it.
 */
export async function attach(id: string, write: (data: string) => void) {
  const waiting = early.get(id);
  early.delete(id);
  let shown = false;
  // What it printed before anyone listened: from wb-agent, unless it's all here.
  writers.set(id, (data) => {
    if (shown) write(data);
    else early.set(id, (early.get(id) ?? '') + data);
  });
  if (waiting === undefined) {
    const { data } = await ask({ method: 'terminal.read', params: { id } }, 'output');
    write(data);
  } else write(waiting);
  shown = true;
  const meanwhile = early.get(id);
  early.delete(id);
  if (meanwhile) write(meanwhile);
  return () => {
    writers.delete(id);
  };
}

export async function openTerminal(cwd: string, cols = 100, rows = 24) {
  const { kind: _, ...terminal } = await ask(
    { method: 'terminal.open', params: { cwd, cols, rows } },
    'terminal',
  );
  setTerminals(terminal.id, terminal);
  // A new terminal's first lines come before anyone listens.
  if (!early.has(terminal.id)) early.set(terminal.id, '');
  return terminal;
}

export function typeIn(id: string, data: string) {
  return ask({ method: 'terminal.input', params: { id, data } }, 'done');
}

export function resize(id: string, cols: number, rows: number) {
  return ask({ method: 'terminal.resize', params: { id, cols, rows } }, 'done');
}

export async function closeTerminal(id: string) {
  await ask({ method: 'terminal.close', params: { id } }, 'done');
}

/** Learns what terminals there are, after wb-agent starts or the page reloads. */
export async function loadTerminals(dir: string) {
  const { list } = await ask({ method: 'terminals.list', params: { dir } }, 'terminals');
  for (const t of list) setTerminals(t.id, t);
  return list;
}
