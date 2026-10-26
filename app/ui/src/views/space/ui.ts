// What each codespace's tab has open, kept while the app runs: its
// editor's tabs and the file shown, the session shown, the terminal shown
// and whether the terminals are hidden, and which folders are open.

import { createStore } from 'solid-js/store';

export type Shown = { id: string } | { file: string } | 'new';

export interface SpaceUi {
  /** Open files, by absolute path. */
  tabs: string[];
  file?: string;
  shown?: Shown;
  terminal?: string;
  termShut: boolean;
  /** Folders open in the tree, relative to the codespace. */
  expanded: string[];
  /** Folders opened or shut by hand, so changes don't reopen them. */
  touched: string[];
}

const [ui, setUi] = createStore<Record<string, SpaceUi>>({});

export { setUi, ui };

/** A codespace's state, made the first time it's asked for. */
export function uiOf(id: string): SpaceUi {
  if (!ui[id]) setUi(id, { tabs: [], termShut: false, expanded: [], touched: [] });
  return ui[id];
}

/** Opens a file in the codespace's editor, as a tab of its own. */
export function openFile(id: string, path: string) {
  const now = uiOf(id);
  setUi(id, { tabs: now.tabs.includes(path) ? now.tabs : [...now.tabs, path], file: path });
}

/** Closes a file's tab; the one beside it is shown if it was. */
export function closeFile(id: string, path: string) {
  const { tabs, file } = uiOf(id);
  const at = tabs.indexOf(path);
  const rest = tabs.filter((tab) => tab !== path);
  setUi(id, { tabs: rest, file: file === path ? rest[Math.min(at, rest.length - 1)] : file });
}

export function showSession(id: string, shown: Shown) {
  setUi(id, 'shown', shown);
}
