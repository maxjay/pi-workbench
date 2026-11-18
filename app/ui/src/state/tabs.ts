// The open tabs. Home is always first and can't close. Every workspace you
// have open has a tab of its own.

import { createSignal } from 'solid-js';

export type TabId = 'home' | `ws:${string}`;

const [tabs, setTabs] = createSignal<TabId[]>(['home']);
const [active, setActive] = createSignal<TabId>('home');

export { active, tabs };

/** The workspace a tab shows, if it shows one. */
export const workspaceOf = (id: TabId) => (id.startsWith('ws:') ? id.slice(3) : undefined);

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
