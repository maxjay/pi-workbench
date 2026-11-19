// The open tabs. Home is always first and can't close. Settings, every
// workspace you have open, every pull request you are reviewing and every
// ticket you have open has a tab of its own.

import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';

export type TabId =
  | 'home'
  | 'settings'
  | `ws:${string}`
  | `pr:${string}:${number}`
  | `wi:${string}:${number}`;

const [tabs, setTabs] = createSignal<TabId[]>(['home']);
const [active, setActive] = createSignal<TabId>('home');

export { active, tabs };

/** What a tab's page says about itself for the row of tabs: its colour, and where its agent stands. */
export interface TabMark {
  hue?: string;
  standing?: 'asking' | 'working';
}

const [marks, setMarks] = createStore<Record<string, TabMark>>({});

export { marks };

export const markTab = (tab: TabId, mark: TabMark) => setMarks(tab, mark);

/** The workspace a tab shows, if it shows one. */
export const workspaceOf = (id: TabId) => (id.startsWith('ws:') ? id.slice(3) : undefined);

/** The pull request a tab shows, and the workspace it is asked from, if it shows one. */
export function pullOf(id: TabId) {
  if (!id.startsWith('pr:')) return undefined;
  const [, workspace, pr] = id.split(':');
  return { workspace, pr: Number(pr) };
}

/** The ticket a tab shows, and the workspace it is asked from, if it shows one. */
export function ticketOf(id: TabId) {
  if (!id.startsWith('wi:')) return undefined;
  const [, workspace, ticket] = id.split(':');
  return { workspace, id: Number(ticket) };
}

/** Opens a tab, or goes to it if it is open. */
export function open(id: TabId) {
  if (!tabs().includes(id)) setTabs([...tabs(), id]);
  setActive(id);
}

export function close(id: TabId) {
  if (id === 'home') return;
  const at = tabs().indexOf(id);
  if (at < 0) return;
  const rest = tabs().filter((tab) => tab !== id);
  setTabs(rest);
  if (active() === id) setActive(rest[Math.max(0, at - 1)]);
}
