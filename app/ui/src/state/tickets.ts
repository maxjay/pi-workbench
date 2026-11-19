// Tickets: Azure DevOps work items, as wbd gets them from dova.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import { health } from './health';

/** What Home's list is narrowed to. Nothing: your own tickets that are not finished. */
const [words, setWords] = createSignal('');

export { words };

let typing: ReturnType<typeof setTimeout> | undefined;

/** Narrows Home's list to tickets whose title has these words, once you stop typing. */
export function findTickets(text: string) {
  clearTimeout(typing);
  typing = setTimeout(() => setWords(text.trim()), 300);
}

const [tickets, { refetch }] = createResource(
  // Nothing to ask until wbd is up and has found dova.
  () => (status() === 'ready' && health()?.dova ? { words: words() } : undefined),
  async (asked) => (await ask({ method: 'tickets.list', params: asked }, 'tickets')).rows,
);

export { tickets };

export const refreshTickets = () => refetch();

/** Counts what was changed on a ticket from here, so its tab knows to ask again. */
const [touched, setTouched] = createSignal(0);

export { touched };

const wrote = () => {
  setTouched(touched() + 1);
  void refetch();
};

export const openTicket = (dir: string, id: number) =>
  ask({ method: 'ticket.open', params: { dir, id } }, 'ticket');

/** Moves a ticket to a state, or assigns it to you or to nobody. */
export async function setTicket(dir: string, id: number, to: { state?: string; mine?: boolean }) {
  const params = { dir, id, state: to.state ?? null, mine: to.mine ?? null };
  await ask({ method: 'ticket.set', params }, 'done');
  wrote();
}

export async function commentOnTicket(dir: string, id: number, text: string) {
  await ask({ method: 'ticket.comment', params: { dir, id, text } }, 'done');
  wrote();
}

/** Files a ticket. Its number is the answer. */
export async function fileTicket(dir: string, title: string, kind: string) {
  const filed = await ask({ method: 'ticket.create', params: { dir, title, kind } }, 'filed');
  wrote();
  return filed.id;
}

/** Starts work on a ticket in a folder. The branch it made is the answer. */
export async function startTicket(dir: string, id: number) {
  const started = await ask({ method: 'ticket.start', params: { dir, id } }, 'started');
  wrote();
  return started.branch;
}

/** What to tell the agent about a ticket, and the folder it works in. */
export const ticketSession = (dir: string, id: number) =>
  ask({ method: 'ticket.session', params: { dir, id } }, 'session');

/** The conversation each ticket's tab last showed, so its session is found again. */
const sessions: Record<string, string> = JSON.parse(localStorage.getItem('ticket-sessions') ?? '{}');

export const sessionOf = (ticket: string): string | undefined => sessions[ticket];

export function keepSession(ticket: string, conversation: string | undefined) {
  if (conversation) sessions[ticket] = conversation;
  else delete sessions[ticket];
  localStorage.setItem('ticket-sessions', JSON.stringify(sessions));
}
