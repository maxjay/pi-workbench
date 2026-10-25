// Pi sessions, as wb-agent reports them: each live one mirrored from the
// changes it sends, saved ones read from Pi's files for looking at without
// starting Pi, and the scratchpads it lists. wb-agent owns them; this keeps
// the latest it said and asks it to act.

import { createEffect, createResource, createRoot, createSignal } from 'solid-js';
import { createStore, produce, reconcile } from 'solid-js/store';
import { ask, onEvent, status } from '../agent';
import type { AskReply } from '../protocol/AskReply';
import type { Session } from '../protocol/Session';

const [live, setLive] = createStore<Record<string, Session>>({});
/** Saved sessions read from their files, by file. Pi isn't running for these. */
const [kept, setKept] = createStore<Record<string, Session>>({});

/** What the Scratchpad tab shows: a live session, or a saved one. */
export type View = { id: string } | { file: string };
const [view, setView] = createSignal<View>();

/** What you were writing in each scratchpad, by its file or live id. */
const [drafts, setDrafts] = createStore<Record<string, string>>({});

export { drafts, kept, live, setDrafts, view };

/** The session the Scratchpad tab shows. */
export function current(): Session | undefined {
  const shown = view();
  if (!shown) return undefined;
  return 'id' in shown ? live[shown.id] : kept[shown.file];
}

/** Where a session's draft is kept: its file once Pi has one, so it survives Pi restarting. */
export const draftKey = (session: Session) => session.state.file ?? session.id;

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
    case 'state': {
      const was = live[id].state.done;
      setLive(id, 'state', reconcile(change.state));
      if (was !== change.state.done) void reloadList();
      break;
    }
    case 'closed': {
      // Still shown: keep what it said, to read until you write in it again.
      const session = live[id];
      const file = session.state.file;
      const shown = view();
      if (file)
        setKept(file, {
          ...session,
          id: '',
          state: { ...session.state, busy: false, ask: null, stopped: null, queued: [] },
        });
      setLive(produce((all) => delete all[id]));
      if (shown && 'id' in shown && shown.id === id) setView(file ? { file } : undefined);
      void reloadList();
      break;
    }
  }
});

// When wb-agent goes away, so do its sessions.
createRoot(() =>
  createEffect(() => {
    if (status() === 'down') {
      setLive(reconcile({}));
      setKept(reconcile({}));
      setView();
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
  asking: boolean;
  stopped: boolean;
  done: boolean;
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
      asking: session.state.ask !== null,
      stopped: session.state.stopped !== null,
      done: session.state.done,
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
      asking: false,
      stopped: false,
      done: s.done,
      updated: s.updated,
    });
  }
  return rows;
}

/** Whether the Scratchpad tab is showing this one. */
export function isShown(pad: Pick<Scratchpad, 'id' | 'file'>) {
  const shown = view();
  if (!shown) return false;
  return 'id' in shown ? pad.id === shown.id : pad.file === shown.file;
}

export { reloadList, saved };

function keep(session: Session) {
  setLive(session.id, reconcile(session));
  setView({ id: session.id });
  void reloadList();
  return session.id;
}

/** Starts a scratchpad in a folder of its own, and asks Pi `text`. */
export async function startScratchpad(text?: string) {
  return keep(await ask({ method: 'session.start', params: { cwd: null, text: text ?? null } }, 'session'));
}

/** Shows a scratchpad: live as it is, saved ones read from Pi's file without starting Pi. */
export async function showScratchpad(pad: Pick<Scratchpad, 'id' | 'file'>) {
  if (pad.id) {
    if (!live[pad.id])
      setLive(pad.id, await ask({ method: 'session.get', params: { id: pad.id } }, 'session'));
    setView({ id: pad.id });
  } else if (pad.file) {
    const file = pad.file;
    setKept(file, reconcile(await ask({ method: 'session.read', params: { file } }, 'session')));
    setView({ file });
  }
}

/** Resumes a saved session, or starts Pi again for one whose Pi stopped. */
export async function reopen(file: string) {
  return keep(await ask({ method: 'session.open', params: { file } }, 'session'));
}

/** The live id for a session, starting Pi for a saved one. */
async function wake(session: Session) {
  if (session.id && live[session.id]) return session.id;
  const file = session.state.file;
  if (!file) throw new Error('This scratchpad has no file to open');
  return reopen(file);
}

/** Sends a message, starting Pi first if the scratchpad isn't live. */
export async function send(session: Session, text: string, later: boolean) {
  const id = await wake(session);
  return ask({ method: 'session.send', params: { id, text, later } }, 'done');
}

/** Stops the run, giving back what was waiting to be sent. */
export async function abort(id: string) {
  return (await ask({ method: 'session.abort', params: { id } }, 'unsent')).texts;
}

/** Takes back a message waiting to be sent. */
export function unqueue(id: string, index: number) {
  return ask({ method: 'session.unqueue', params: { id, index } }, 'done');
}

export function answer(id: string, question: string, reply: AskReply) {
  return ask({ method: 'session.answer', params: { id, ask: question, reply } }, 'done');
}

export async function availableModels(id: string) {
  return (await ask({ method: 'session.models', params: { id } }, 'available')).models;
}

export async function setModel(session: Session, provider: string, model: string) {
  const id = await wake(session);
  return ask({ method: 'session.setModel', params: { id, provider, model } }, 'done');
}

export async function setThinking(session: Session, level: string) {
  const id = await wake(session);
  return ask({ method: 'session.setThinking', params: { id, level } }, 'done');
}

/** Marks a scratchpad done, which stops its Pi, or brings it back. */
export async function setDone(file: string, done: boolean) {
  await ask({ method: 'sessions.setDone', params: { file, done } }, 'done');
  if (kept[file]) setKept(file, 'state', 'done', done);
  await reloadList();
}

/** A file Pi wrote, by its absolute path, so a saved session's files read too. */
export function readFile(session: Session, path: string) {
  return ask({ method: 'file.read', params: { session: session.id, path } }, 'file');
}
