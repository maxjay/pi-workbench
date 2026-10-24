// Pi sessions, as wb-agent reports them: each live one mirrored from the
// changes it sends, and the scratchpads it lists. wb-agent owns them; this
// keeps the latest it said and asks it to act.

import { createEffect, createResource, createRoot, createSignal } from 'solid-js';
import { createStore, produce, reconcile } from 'solid-js/store';
import { ask, onEvent, status } from '../agent';
import type { AskReply } from '../protocol/AskReply';
import type { Session } from '../protocol/Session';

const [live, setLive] = createStore<Record<string, Session>>({});
/** The live session the Scratchpad tab shows. */
const [selected, setSelected] = createSignal<string>();

export { live, selected };

const [saved, { refetch: reloadList }] = createResource(
  () => status() === 'ready',
  async () => (await ask({ method: 'sessions.list' }, 'sessions')).sessions,
);

onEvent((event) => {
  if (event.event !== 'session') return;
  const { id, change } = event;
  // Changes from before the session's snapshot arrived are in the snapshot.
  if (!live[id]) return;
  switch (change.change) {
    case 'entry':
      setLive(id, 'entries', change.index, change.entry);
      break;
    case 'entries':
      setLive(id, 'entries', reconcile(change.entries));
      break;
    case 'state':
      setLive(id, 'state', reconcile(change.state));
      break;
    case 'closed':
      setLive(produce((all) => delete all[id]));
      if (selected() === id) setSelected();
      void reloadList();
      break;
  }
});

// When wb-agent goes away, so do its sessions.
createRoot(() =>
  createEffect(() => {
    if (status() === 'down') {
      setLive(reconcile({}));
      setSelected();
    }
  }),
);

/** A scratchpad in the list: live, or saved in Pi's store. */
export interface Scratchpad {
  key: string;
  id?: string;
  file?: string;
  title: string;
  busy: boolean;
  updated: number;
}

/** Live scratchpads first, newest first, then saved ones. */
export function scratchpads(): Scratchpad[] {
  const list = saved.error ? [] : (saved.latest ?? []);
  const rows: Scratchpad[] = Object.values(live)
    .map((session) => ({
      key: session.id,
      id: session.id,
      file: session.state.file ?? undefined,
      title: session.state.title,
      busy: session.state.busy,
      updated: list.find((s) => s.id === session.id)?.updated ?? Date.now(),
    }))
    .reverse();
  for (const s of list) {
    if (s.id || rows.some((row) => row.file && row.file === s.file)) continue;
    rows.push({
      key: s.file ?? s.title,
      file: s.file ?? undefined,
      title: s.title,
      busy: false,
      updated: s.updated,
    });
  }
  return rows;
}

export { reloadList, saved };

function keep(session: Session) {
  setLive(session.id, reconcile(session));
  setSelected(session.id);
  void reloadList();
  return session.id;
}

/** Starts a scratchpad in a folder of its own, and asks Pi `text`. */
export async function startScratchpad(text?: string) {
  return keep(await ask({ method: 'session.start', params: { cwd: null, text: text ?? null } }, 'session'));
}

/** Shows a scratchpad, resuming it from Pi's store if it isn't live. */
export async function showScratchpad(pad: Pick<Scratchpad, 'id' | 'file'>) {
  if (pad.id) {
    if (!live[pad.id])
      setLive(pad.id, await ask({ method: 'session.get', params: { id: pad.id } }, 'session'));
    setSelected(pad.id);
  } else if (pad.file) {
    await reopen(pad.file);
  }
}

/** Resumes a saved session, or starts Pi again for one whose Pi stopped. */
export async function reopen(file: string) {
  return keep(await ask({ method: 'session.open', params: { file } }, 'session'));
}

export function send(id: string, text: string, later: boolean) {
  return ask({ method: 'session.send', params: { id, text, later } }, 'done');
}

export function abort(id: string) {
  return ask({ method: 'session.abort', params: { id } }, 'done');
}

export function answer(id: string, question: string, reply: AskReply) {
  return ask({ method: 'session.answer', params: { id, ask: question, reply } }, 'done');
}

export function setModel(id: string, provider: string, model: string) {
  return ask({ method: 'session.setModel', params: { id, provider, model } }, 'done');
}

export function setThinking(id: string, level: string) {
  return ask({ method: 'session.setThinking', params: { id, level } }, 'done');
}

/** Stops Pi for a session; it stays in the list, saved. */
export function closeSession(id: string) {
  return ask({ method: 'session.close', params: { id } }, 'done');
}

export function readFile(session: string, path: string) {
  return ask({ method: 'file.read', params: { session, path } }, 'file');
}
