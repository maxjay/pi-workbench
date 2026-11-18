// The one connection to wbd. Everything the page knows comes through here:
// `ask` sends a call and resolves with its answer, `onEvent` hears what wbd
// says on its own. JSON-RPC 2.0, one object per line.
//
// In the app the lines travel over Tauri. In a plain browser (`npm run dev`)
// they travel over the dev server's WebSocket. Nothing else needs to know which.

import { createSignal } from 'solid-js';
import type { Answer } from './protocol/Answer';
import type { Call } from './protocol/Call';
import type { Event } from './protocol/Event';
import type { Failure } from './protocol/Failure';
import type { FailureKind } from './protocol/FailureKind';

export type Status = 'connecting' | 'ready' | 'down';

/** A call that got no answer: wbd refused it (then `kind` says why), or wbd went away. */
export class DaemonError extends Error {
  constructor(
    message: string,
    readonly kind?: FailureKind,
  ) {
    super(message);
  }
}

/** One line from wbd: an answer, a refusal, or an event. */
interface Line {
  id?: number | null;
  result?: Answer;
  error?: Failure;
  method?: string;
  params?: Event;
}

const [status, setStatus] = createSignal<Status>('connecting');
/** Why wbd is down, while it is. */
const [stopReason, setStopReason] = createSignal<string>();

export { status, stopReason };

const waiting = new Map<number, { resolve: (answer: Answer) => void; reject: (err: DaemonError) => void }>();
const listeners = new Set<(event: Event) => void>();
let nextId = 1;
/** Sends one line, once there is a way to. Until then lines wait in `unsent`. */
let send: ((line: string) => void) | undefined;
const unsent: string[] = [];

/** Sends a call and resolves with the answer of the kind it expects. */
export async function ask<K extends Answer['kind']>(
  call: Call,
  kind: K,
): Promise<Extract<Answer, { kind: K }>> {
  const id = nextId++;
  const line = JSON.stringify({ jsonrpc: '2.0', id, ...call });
  const answer = await new Promise<Answer>((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    if (send) send(line);
    else unsent.push(line);
  });
  if (answer.kind !== kind) throw new DaemonError(`expected ${kind}, got ${answer.kind}`);
  return answer as Extract<Answer, { kind: K }>;
}

export function onEvent(listener: (event: Event) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Starts wbd and the connection to it. Call it again to start over. */
export async function connect() {
  setStatus('connecting');
  setStopReason();
  try {
    send = '__TAURI_INTERNALS__' in window ? await overTauri() : await overSocket();
    for (const line of unsent.splice(0)) send(line);
  } catch (err) {
    down(err instanceof Error ? err.message : String(err));
  }
}

function receive(text: string) {
  const line = JSON.parse(text) as Line;
  if (line.method === 'event' && line.params) {
    if (line.params.event === 'ready') setStatus('ready');
    for (const listener of listeners) listener(line.params);
    return;
  }
  if (typeof line.id !== 'number') return;
  const call = waiting.get(line.id);
  waiting.delete(line.id);
  if (line.error) call?.reject(new DaemonError(line.error.message, line.error.data.kind));
  else if (line.result) call?.resolve(line.result);
}

/** wbd stopped: nothing asked will be answered. */
function down(reason: string) {
  send = undefined;
  setStatus('down');
  setStopReason(reason);
  for (const call of waiting.values()) call.reject(new DaemonError(reason));
  waiting.clear();
}

/** The app's events are listened for once, however often wbd is started. */
let hearing = false;

async function overTauri() {
  const { invoke } = await import('@tauri-apps/api/core');
  if (!hearing) {
    hearing = true;
    const { listen } = await import('@tauri-apps/api/event');
    await listen<string>('wbd-line', (e) => receive(e.payload));
    await listen<string>('wbd-exit', (e) => down(e.payload));
  }
  await invoke('daemon_start');
  return (line: string) => {
    invoke('daemon_send', { line }).catch((err) => down(String(err)));
  };
}

function overSocket() {
  return new Promise<(line: string) => void>((resolve, reject) => {
    const socket = new WebSocket(`ws://${location.host}/wbd`);
    socket.onopen = () => resolve((line) => socket.send(line));
    socket.onmessage = (e) => receive(String(e.data));
    socket.onerror = () => reject(new Error("couldn't reach the dev server"));
    socket.onclose = (e) => down(e.reason || 'wbd stopped');
  });
}
