// The open tabs, as in a browser: Home is always first and can't close;
// the rest open, focus and close. Home, the Scratchpad and Settings have a
// tab each; every task, codespace and web page you have open has one of
// its own, and a new task one while it's being made.

import { createSignal } from 'solid-js';
import { pageById, pageTitle, tabClosed } from './browser';
import { spaceById } from './codespaces';
import { taskById } from './tasks';

export type TabId =
  | 'home'
  | 'scratch'
  | 'settings'
  | 'newtask'
  | `task:${string}`
  | `space:${string}`
  | `web:${string}`;
export type TabKind = 'home' | 'scratch' | 'settings' | 'task' | 'space' | 'web';

export const kindOf = (id: TabId): TabKind =>
  id === 'newtask' || id.startsWith('task:')
    ? 'task'
    : id.startsWith('space:')
      ? 'space'
      : id.startsWith('web:')
        ? 'web'
        : (id as TabKind);

/** The task a tab shows, if it shows one. */
export const taskOf = (id: TabId) => (id.startsWith('task:') ? id.slice(5) : undefined);

/** The codespace a tab shows, if it shows one. */
export const spaceOf = (id: TabId) => (id.startsWith('space:') ? id.slice(6) : undefined);

const fixed: Record<string, string> = {
  home: 'Home',
  scratch: 'Scratchpad',
  settings: 'Settings',
  newtask: 'New task',
};

export function title(id: TabId) {
  const task = taskOf(id);
  if (task) return taskById(task)?.task.name ?? 'Task';
  const space = spaceOf(id);
  if (space) return spaceById(space)?.space.name ?? 'Codespace';
  if (id.startsWith('web:')) return pageTitle(pageById(id.slice(4)));
  return fixed[id];
}

const [tabs, setTabs] = createSignal<TabId[]>(['home']);
const [active, setActive] = createSignal<TabId>('home');

export { active, tabs };

/** Opens a tab and goes to it; with `focus: false`, opens it behind the one you're on. */
export function open(id: TabId, options: { focus?: boolean } = {}) {
  if (!tabs().includes(id)) setTabs([...tabs(), id]);
  if (options.focus !== false) setActive(id);
}

export function close(id: TabId) {
  if (id === 'home') return;
  const list = tabs();
  const at = list.indexOf(id);
  if (at < 0) return;
  const rest = list.filter((tab) => tab !== id);
  setTabs(rest);
  if (active() === id) setActive(rest[Math.max(0, at - 1)]);
  if (id.startsWith('web:')) tabClosed(id.slice(4));
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
