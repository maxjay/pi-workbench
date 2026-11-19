// Conversations with an agent: the ones open in the page, kept up to date
// from wbd's events, and each folder's list of them.

import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { ask, onEvent } from '../daemon';
import type { Agent } from '../protocol/Agent';
import type { Conversation } from '../protocol/Conversation';
import type { Entry } from '../protocol/Entry';
import type { Offer } from '../protocol/Offer';

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

/**
 * Starts a conversation with what you wrote, on the model and effort last picked. Returns its id.
 * `brief` is what the agent is told before anything you write.
 */
export async function start(agent: Agent, cwd: string, text: string, brief: string | null = null) {
  const params = { agent, cwd, text, ...picks[agent], brief };
  const view = await ask({ method: 'conversation.start', params }, 'conversation');
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

/**
 * What the agent ends its turn with: the last thing it wrote after entry `from`.
 * Resolves when the turn is over. Rejects if the agent stopped.
 */
export function turnEnd(id: string, from: number): Promise<string> {
  const last = () =>
    open[id]?.entries
      .slice(from)
      .filter((entry) => entry.kind === 'text')
      .at(-1)?.text ?? '';
  return new Promise((resolve, reject) => {
    // The turn may have started, or even ended, before this listens.
    let started = open[id]?.state.busy ?? false;
    if (!started && last()) return resolve(last());
    const off = onEvent((event) => {
      if (event.event !== 'conversation' || event.id !== id) return;
      const state = open[id]?.state;
      if (!state) return;
      started ||= state.busy;
      if (state.stopped) {
        off();
        reject(new Error(state.stopped));
      } else if (started && !state.busy && !state.ask) {
        off();
        resolve(last());
      }
    });
  });
}

/**
 * Says something in a conversation, or starts one in `cwd` when there is
 * none. Gives the conversation's id at once, and what the agent ends its
 * turn on when it has.
 */
export async function converse(cwd: string, brief: string | null, id: string | undefined, text: string) {
  if (!id) {
    const started = await start(defaultAgent(), cwd, text, brief);
    return { id: started, answer: turnEnd(started, 0) };
  }
  await show(id);
  if (open[id]?.state.busy) throw new Error('The agent is busy. Wait for it, or stop it.');
  const from = open[id]?.entries.length ?? 0;
  await send(id, text);
  return { id, answer: turnEnd(id, from) };
}

export const stop = (id: string) => ask({ method: 'conversation.stop', params: { id } }, 'done');

/** Answers what the agent asked. `answer` is null when you dismiss the question. */
export const answer = (id: string, question: string, answer: string | null) =>
  ask({ method: 'conversation.answer', params: { id, ask: question, answer } }, 'done');

export const agentName: Record<Agent, string> = { pi: 'Pi', claude: 'Claude Code' };

/** The agent a new conversation starts with. Kept in the window's own storage. */
const [defaultAgent, pickDefault] = createSignal<Agent>(
  localStorage.getItem('agent') === 'claude' ? 'claude' : 'pi',
);

export { defaultAgent };

export function setDefaultAgent(agent: Agent) {
  localStorage.setItem('agent', agent);
  pickDefault(agent);
}

/** Where a folder's conversations stand, for its tab: asking beats working. */
export function standing(cwd: string): 'asking' | 'working' | undefined {
  const here = [...(lists[cwd] ?? []), ...Object.values(open).map((o) => o.state)].filter(
    (c) => c.cwd === cwd,
  );
  if (here.some((c) => c.ask)) return 'asking';
  return here.some((c) => c.busy) ? 'working' : undefined;
}

/** A model and an effort. Null is the agent's own. */
export interface Picked {
  model: string | null;
  effort: string | null;
}

/** What each agent offers, as last asked. */
const [offers, setOffers] = createStore<Partial<Record<Agent, Offer>>>({});

export { offers };

/** Asks an agent what it offers: its models, their efforts, whether it is signed in. */
export async function askOffer(agent: Agent) {
  const offer = await ask({ method: 'agent.offer', params: { agent } }, 'offer');
  setOffers(agent, offer);
  return offer;
}

/** What a new conversation with each agent starts on: what you last picked. Kept in the window's own storage. */
const [picks, setPicks] = createStore<Record<Agent, Picked>>(storedPicks());

export { picks };

function storedPicks(): Record<Agent, Picked> {
  const none = { model: null, effort: null };
  try {
    const stored = JSON.parse(localStorage.getItem('picks') ?? '{}');
    return { pi: { ...none, ...stored.pi }, claude: { ...none, ...stored.claude } };
  } catch {
    return { pi: none, claude: { ...none } };
  }
}

export function pick(agent: Agent, picked: Picked) {
  setPicks(agent, picked);
  localStorage.setItem('picks', JSON.stringify(picks));
}

/** Changes a conversation's model and effort. New conversations with its agent start on them too. */
export async function setPicked(id: string, agent: Agent, picked: Picked) {
  await ask({ method: 'conversation.set', params: { id, ...picked } }, 'done');
  if (open[id]) setOpen(id, 'state', picked);
  pick(agent, picked);
}
