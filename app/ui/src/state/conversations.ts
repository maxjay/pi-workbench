// Conversations with an agent: the ones open in the page, kept up to date
// from wbd's events, and each folder's list of them.

import { createStore } from 'solid-js/store';
import { ask, onEvent } from '../daemon';
import type { Agent } from '../protocol/Agent';
import type { Conversation } from '../protocol/Conversation';
import type { Entry } from '../protocol/Entry';

export interface Open {
  state: Conversation;
  entries: Entry[];
}

/** The conversations the page has opened, by id. */
const [open, setOpen] = createStore<Record<string, Open>>({});
/** Each folder's conversations, the latest first, as last listed. */
const [lists, setLists] = createStore<Record<string, Conversation[]>>({});

export { lists, open };

onEvent((event) => {
  if (event.event === 'ready') {
    // A new wbd runs no agents: what was working has stopped.
    for (const id of Object.keys(open)) setOpen(id, 'state', { busy: false, ask: null });
    return;
  }
  if (event.event !== 'conversation') return;
  for (const change of event.changes) {
    if (change.kind === 'entry') {
      if (open[event.id]) setOpen(event.id, 'entries', change.index, change.entry);
      continue;
    }
    if (open[event.id]) setOpen(event.id, 'state', change.state);
    const list = lists[change.state.cwd];
    const at = list?.findIndex((c) => c.id === event.id) ?? -1;
    if (at >= 0) setLists(change.state.cwd, at, change.state);
  }
});

/** Asks wbd for a folder's conversations again. */
export async function refreshList(cwd: string) {
  const { list } = await ask({ method: 'conversations.list', params: { cwd } }, 'conversations');
  setLists(cwd, list);
}

/** Starts a conversation with what you wrote. Returns its id. */
export async function start(agent: Agent, cwd: string, text: string) {
  const view = await ask({ method: 'conversation.start', params: { agent, cwd, text } }, 'conversation');
  setOpen(view.state.id, { state: view.state, entries: view.entries });
  setLists(cwd, [view.state, ...(lists[cwd] ?? [])]);
  return view.state.id;
}

/** Reads a conversation into the page, unless it is there already. */
export async function show(id: string) {
  if (open[id]) return;
  const view = await ask({ method: 'conversation.open', params: { id } }, 'conversation');
  setOpen(id, { state: view.state, entries: view.entries });
}

export const send = (id: string, text: string) =>
  ask({ method: 'conversation.send', params: { id, text } }, 'done');

export const stop = (id: string) => ask({ method: 'conversation.stop', params: { id } }, 'done');

/** Answers what the agent asked. `answer` is null when you dismiss the question. */
export const answer = (id: string, question: string, answer: string | null) =>
  ask({ method: 'conversation.answer', params: { id, ask: question, answer } }, 'done');

export const agentName: Record<Agent, string> = { pi: 'Pi', claude: 'Claude Code' };

/** Where a folder's conversations stand, for its tab: asking beats working. */
export function standing(cwd: string): 'asking' | 'working' | undefined {
  const here = [...(lists[cwd] ?? []), ...Object.values(open).map((o) => o.state)].filter(
    (c) => c.cwd === cwd,
  );
  if (here.some((c) => c.ask)) return 'asking';
  return here.some((c) => c.busy) ? 'working' : undefined;
}
