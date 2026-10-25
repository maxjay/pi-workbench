// The open tabs, as in a browser: Home is always first and can't close;
// the rest open, focus and close. Home, the Scratchpad and Settings have a
// tab each; every task has one of its own, and a new task one while it's
// being made.

import { createSignal } from 'solid-js';
import { taskById } from './tasks';

export type TabId = 'home' | 'scratch' | 'settings' | 'newtask' | `task:${string}`;
export type TabKind = 'home' | 'scratch' | 'settings' | 'task';

export const kindOf = (id: TabId): TabKind =>
  id === 'newtask' || id.startsWith('task:') ? 'task' : (id as TabKind);

/** The task a tab shows, if it shows one. */
export const taskOf = (id: TabId) => (id.startsWith('task:') ? id.slice(5) : undefined);

const fixed: Record<string, string> = {
  home: 'Home',
  scratch: 'Scratchpad',
  settings: 'Settings',
  newtask: 'New task',
};

export function title(id: TabId) {
  const task = taskOf(id);
  return task ? (taskById(task)?.task.name ?? 'Task') : fixed[id];
}

const [tabs, setTabs] = createSignal<TabId[]>(['home']);
const [active, setActive] = createSignal<TabId>('home');

export { active, tabs };

export function open(id: TabId) {
  if (!tabs().includes(id)) setTabs([...tabs(), id]);
  setActive(id);
}

export function close(id: TabId) {
  if (id === 'home') return;
  const list = tabs();
  const at = list.indexOf(id);
  if (at < 0) return;
  const rest = list.filter((tab) => tab !== id);
  setTabs(rest);
  if (active() === id) setActive(rest[Math.max(0, at - 1)]);
}

/** Puts one tab in another's place: a new task's, once it's made. */
export function replace(from: TabId, to: TabId) {
  setTabs(
    tabs().includes(to)
      ? tabs().filter((tab) => tab !== from)
      : tabs().map((tab) => (tab === from ? to : tab)),
  );
  setActive(to);
}

export function focus(id: TabId) {
  setActive(id);
}
