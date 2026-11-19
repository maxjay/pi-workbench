// What each open workspace's tab is showing: which list is on the left,
// which folders are open in the tree, and the tabs in the middle. Kept
// while the app runs.
//
// A middle tab is a string. A file's tab is its absolute path, which always
// starts with `/`. `newpr` is the form for a new pull request, and `newpr:` with a folder
// after it the form for one repo of a workspace that holds several.

import { createStore } from 'solid-js/store';

export type Left = 'files' | 'changes';

export interface WorkspaceUi {
  left: Left;
  /** Folders open in the tree, as paths relative to the workspace. */
  expanded: string[];
  tabs: string[];
  /** The tab shown, if any is open. */
  current: string | undefined;
  /** The conversation with the agent that is shown. Undefined for a new one. */
  conversation: string | undefined;
}

const [uis, setUis] = createStore<Record<string, WorkspaceUi>>({});

export function uiOf(id: string): WorkspaceUi {
  if (!uis[id])
    setUis(id, {
      left: 'files',
      expanded: [],
      tabs: [],
      current: undefined,
      conversation: undefined,
    });
  return uis[id];
}

export const isFile = (tab: string) => tab.startsWith('/');

/** A folder's path with the home folder as `~`. */
export const shortPath = (path: string) => path.replace(/^\/home\/[^/]+/, '~');

/** The last part of a path. */
export const nameOf = (path: string) => path.split('/').pop() ?? path;

/** What a middle tab is called on its tab. */
export function labelOf(tab: string) {
  return isFile(tab) ? nameOf(tab) : 'New pull request';
}

export function setLeft(id: string, left: Left) {
  setUis(id, 'left', left);
}

export function toggleFolder(id: string, folder: string) {
  const open = uiOf(id).expanded;
  setUis(id, 'expanded', open.includes(folder) ? open.filter((f) => f !== folder) : [...open, folder]);
}

/** Opens a tab, or goes to it if it is open. */
export function openTab(id: string, tab: string) {
  const ui = uiOf(id);
  if (!ui.tabs.includes(tab)) setUis(id, 'tabs', [...ui.tabs, tab]);
  setUis(id, 'current', tab);
}

export function closeTab(id: string, tab: string) {
  const ui = uiOf(id);
  const at = ui.tabs.indexOf(tab);
  if (at < 0) return;
  const rest = ui.tabs.filter((t) => t !== tab);
  setUis(id, 'tabs', rest);
  if (ui.current === tab) setUis(id, 'current', rest[Math.min(at, rest.length - 1)]);
}

export function setConversation(id: string, conversation: string | undefined) {
  // Home sets this for a scratchpad before its tab has ever been shown.
  uiOf(id);
  setUis(id, 'conversation', conversation);
}
