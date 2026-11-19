// Tickets: Azure DevOps work items, as wbd gets them from dova.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import type { PartEdit } from '../protocol/PartEdit';
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

/** Changes a ticket's title, parts of its text, or both. */
export async function editTicket(dir: string, id: number, title: string | null, parts: PartEdit[]) {
  await ask({ method: 'ticket.edit', params: { dir, id, title, parts } }, 'done');
  wrote();
}

/** Sets one field of a ticket. With no value, the field is emptied. */
export async function setField(dir: string, id: number, field: string, value: string | null) {
  await ask({ method: 'ticket.field', params: { dir, id, field, value } }, 'done');
  wrote();
}

/** Files a ticket under another one, of the type that goes under it. */
export async function addChild(dir: string, id: number, title: string) {
  const filed = await ask({ method: 'ticket.child', params: { dir, id, title } }, 'filed');
  wrote();
  return filed.id;
}

/** Takes a ticket out from under another one. Neither is deleted. */
export async function unlinkChild(dir: string, id: number, child: number) {
  await ask({ method: 'ticket.unlink', params: { dir, id, child } }, 'done');
  wrote();
}

/** Ticks a ticket off, or takes the tick back. */
export async function tickOff(dir: string, id: number, done: boolean) {
  await ask({ method: 'ticket.done', params: { dir, id, done } }, 'done');
  wrote();
}

export async function commentOnTicket(dir: string, id: number, text: string) {
  await ask({ method: 'ticket.comment', params: { dir, id, text } }, 'done');
  wrote();
}

/** Files a ticket. Its number is the answer. */
export async function fileTicket(dir: string, title: string, kind: string, team: string | null = null) {
  const filed = await ask({ method: 'ticket.create', params: { dir, title, kind, team } }, 'filed');
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
