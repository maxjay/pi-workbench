// The one conversation a pull request's tab or a ticket's tab has with
// the agent. What is asked from the tab, under a line or in a thread, is a
// turn of that conversation, one at a time: a question asked while the
// agent works waits for it.

import { createSignal } from 'solid-js';
import type { Agent } from '../../protocol/Agent';
import {
  agentName,
  open as conversations,
  defaultAgent,
  send,
  show,
  start,
  turnEnd,
} from '../../state/conversations';

/** What is said to you alone about something, and who says it. */
export interface Host {
  /** Names the pull request or the ticket in the window's own storage. */
  key: string;
  /** The agent's name, as it is written beside what it says. */
  agent: () => string;
  /** Asks the agent about a thread or lines. The question and the answer show there, for you alone. */
  ask: (talk: string, question: string) => void;
  /** Has the agent write a comment. It waits there, not posted. */
  draft: (talk: string) => void;
}

export const why = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** `session` is the folder the agent works in and what it is told first, once they are ready. */
export function createTalker(session: Promise<{ cwd: string; brief: string }>) {
  const [conversation, setConversation] = createSignal<string>();
  /** The agent a new conversation starts with. */
  const [agent, setAgent] = createSignal<Agent>(defaultAgent());
  const pause = (ms: number) => new Promise((done) => setTimeout(done, ms));
  let queue: Promise<unknown> = Promise.resolve();

  /** Says something in the conversation and gives what the agent ends its turn on. */
  const tell = (prompt: string): Promise<string> => {
    const turn = async () => {
      const ready = await session;
      const id = conversation();
      if (!id) {
        const started = await start(agent(), ready.cwd, prompt, ready.brief);
        setConversation(started);
        return turnEnd(started, 0);
      }
      await show(id);
      // Something written in the chat is being answered: this waits its turn.
      while (conversations[id]?.state.busy || conversations[id]?.state.ask) await pause(300);
      const from = conversations[id]?.entries.length ?? 0;
      await send(id, prompt);
      return turnEnd(id, from);
    };
    const next = queue.then(turn, turn);
    queue = next.catch(() => undefined);
    return next;
  };

  return {
    conversation,
    setConversation,
    agent,
    setAgent,
    /** The name of the agent talked to: this conversation's, or the one a new one starts with. */
    name: () => {
      const id = conversation();
      return agentName[(id ? conversations[id]?.state.agent : undefined) ?? agent()];
    },
    /** Where the conversation stands, for the tab: waiting on you beats working. */
    standing: (): 'asking' | 'working' | undefined => {
      const id = conversation();
      const state = id ? conversations[id]?.state : undefined;
      return state?.ask ? 'asking' : state?.busy ? 'working' : undefined;
    },
    tell,
  };
}
