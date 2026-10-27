// The one connection to wb-agent. Everything the UI knows comes through
// here: `ask` sends a request and resolves with its answer; `onEvent`
// hears what wb-agent says on its own.
//
// In the app the lines travel over Tauri; in a plain browser (`npm run dev`)
// over the dev bridge's WebSocket. Nothing else needs to know which.

import { createSignal } from 'solid-js';
import type { Answer } from './protocol/Answer';
import type { Call } from './protocol/Call';
import type { Event } from './protocol/Event';
import type { Failure } from './protocol/Failure';
import type { FromAgent } from './protocol/FromAgent';

export type Status = 'connecting' | 'ready' | 'down';

/** A request wb-agent refused, or couldn't answer because it went away. */
export class AgentError extends Error {
  constructor(readonly failure: Failure) {
    super(failure.message);
  }
}

interface Transport {
  send(line: string): void;
}

const [status, setStatus] = createSignal<Status>('connecting');
/** Why wb-agent went down, while it's down. */
const [stopReason, setStopReason] = createSignal<string>();

export { status, stopReason };

const inApp = '__TAURI_INTERNALS__' in window;

const pending = new Map<number, { resolve: (answer: Answer) => void; reject: (err: AgentError) => void }>();
const listeners = new Set<(event: Event) => void>();
let nextId = 1;
let transport: Transport | undefined;
const queued: string[] = [];

/** Sends a call and resolves with the answer of the kind it expects. */
export function ask<K extends Answer['kind']>(call: Call, kind: K): Promise<Extract<Answer, { kind: K }>> {
  const id = nextId++;
  const line = JSON.stringify({ id, ...call });
  const answer = new Promise<Answer>((resolve, reject) => pending.set(id, { resolve, reject }));
  if (transport) transport.send(line);
  else queued.push(line);
  return answer.then((answer) => {
    if (answer.kind !== kind) {
      throw new AgentError({ code: 'badRequest', message: `expected ${kind}, got ${answer.kind}` });
    }
    return answer as Extract<Answer, { kind: K }>;
  });
}

export function onEvent(listener: (event: Event) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function receive(line: string) {
  let message: FromAgent;
  try {
    message = JSON.parse(line) as FromAgent;
  } catch {
    console.error('wb-agent sent a line that isn’t JSON:', line);
    return;
  }
  if (message.type === 'event') {
    if (message.event === 'ready') {
      setStatus('ready');
      setStopReason();
    }
    for (const listener of listeners) listener(message);
    return;
  }
  // The app carries these to the browser; they're never the page's.
  if (message.type === 'tunnel') return;
  const waiting = pending.get(message.id);
  if (!waiting) return;
  pending.delete(message.id);
  if ('ok' in message) waiting.resolve(message.ok);
  else waiting.reject(new AgentError(message.error));
}

function lost(reason: string) {
  setStatus('down');
  setStopReason(reason);
  transport = undefined;
  for (const waiting of pending.values()) waiting.reject(new AgentError({ code: 'io', message: reason }));
  pending.clear();
}

function opened(next: Transport) {
  transport = next;
  for (const line of queued.splice(0)) next.send(line);
}

/** Asks the app to start wb-agent; if it can't, says why. */
async function startInApp() {
  const { invoke } = await import('@tauri-apps/api/core');
  opened({ send: (line) => void invoke('agent_send', { line }).catch((err) => lost(String(err))) });
  try {
    await invoke('agent_start');
  } catch (err) {
    lost(String(err));
  }
}

/** Connects once, at start-up. */
export async function connect() {
  if (inApp) {
    const { listen } = await import('@tauri-apps/api/event');
    await listen<string>('agent-line', (e) => receive(e.payload));
    await listen<string>('agent-exit', (e) => lost(e.payload));
    await startInApp();
    return;
  }
  const socket = new WebSocket(`ws://${location.host}/agent`);
  socket.addEventListener('open', () => opened({ send: (line) => socket.send(line) }));
  socket.addEventListener('message', (e) => receive(String(e.data)));
  socket.addEventListener('close', (e) => lost(e.reason || 'wb-agent went away'));
}

/** Starts wb-agent again after it went away. */
export async function reconnect() {
  setStatus('connecting');
  if (inApp) await startInApp();
  else await connect();
}
