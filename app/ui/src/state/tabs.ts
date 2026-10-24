// The open tabs, as in a browser: Home is always first and can't close;
// the rest open, focus and close. Each kind of page has one tab for now.

import { createSignal } from 'solid-js';

export type TabKind = 'home' | 'settings';

export interface Tab {
  kind: TabKind;
  title: string;
}

const titles: Record<TabKind, string> = { home: 'Home', settings: 'Settings' };

const [tabs, setTabs] = createSignal<Tab[]>([{ kind: 'home', title: titles.home }]);
const [active, setActive] = createSignal<TabKind>('home');

export { active, tabs };

export function open(kind: TabKind) {
  if (!tabs().some((tab) => tab.kind === kind)) setTabs([...tabs(), { kind, title: titles[kind] }]);
  setActive(kind);
}

export function close(kind: TabKind) {
  if (kind === 'home') return;
  const list = tabs();
  const at = list.findIndex((tab) => tab.kind === kind);
  if (at < 0) return;
  const rest = list.filter((tab) => tab.kind !== kind);
  setTabs(rest);
  if (active() === kind) setActive(rest[Math.max(0, at - 1)].kind);
}

export function focus(kind: TabKind) {
  setActive(kind);
}
