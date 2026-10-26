// The codespaces, as wb-agent keeps them: folders in WSL opened in the
// workbench, each with its git, what runs in its terminals and its
// sessions. Asked for again when any of that may have changed, and now and
// then, as git moves on without telling anyone.

import { createResource, createRoot, onCleanup } from 'solid-js';
import { ask, onEvent, status } from '../agent';
import type { Codespace } from '../protocol/Codespace';
import type { CodespaceView } from '../protocol/CodespaceView';

const [list, { refetch, mutate }] = createResource(
  () => status() === 'ready',
  async () => (await ask({ method: 'codespaces.list' }, 'codespaces')).list,
);

export function codespaces(): CodespaceView[] {
  return list.latest ?? [];
}

export function spaceById(id: string): CodespaceView | undefined {
  return codespaces().find((view) => view.space.id === id);
}

/** The codespace a folder is in, the innermost when one is inside another. */
export function spaceHolding(path: string): Codespace | undefined {
  return codespaces()
    .map((view) => view.space)
    .filter((space) => path === space.path || path.startsWith(`${space.path}/`))
    .sort((a, b) => b.path.length - a.path.length)[0];
}

let soon: ReturnType<typeof setTimeout> | undefined;

/** Asks again shortly, once, however many things changed meanwhile. */
export function reloadCodespaces() {
  clearTimeout(soon);
  soon = setTimeout(() => void refetch(), 400);
}

// What runs in a terminal, and sessions starting, asking and stopping, show on Home.
onEvent((event) => {
  if (event.event === 'terminals') reloadCodespaces();
  if (event.event === 'session' && (event.change.change === 'state' || event.change.change === 'closed'))
    reloadCodespaces();
});

createRoot(() => {
  const every = setInterval(() => {
    if (status() === 'ready' && document.visibilityState === 'visible') void refetch();
  }, 20_000);
  onCleanup(() => clearInterval(every));
});

function put(view: CodespaceView) {
  const rest = codespaces().filter((other) => other.space.id !== view.space.id);
  mutate([view, ...rest]);
}

/** Opens a folder as a codespace, or opens it again. `~` is home. */
export async function openFolder(path: string): Promise<CodespaceView> {
  const { kind: _, ...view } = await ask({ method: 'codespaces.open', params: { path } }, 'codespace');
  put(view);
  reloadCodespaces();
  return view;
}

export async function setPinned(id: string, pinned: boolean) {
  mutate((await ask({ method: 'codespaces.setPinned', params: { id, pinned } }, 'codespaces')).list);
}

/** Takes a codespace off the list. Its folder and its sessions stay. */
export async function forget(id: string) {
  mutate((await ask({ method: 'codespaces.forget', params: { id } }, 'codespaces')).list);
}

/** The folders in a folder, for picking one to open. */
export function foldersIn(dir: string) {
  return ask({ method: 'folders.list', params: { dir } }, 'folders');
}

export { list as codespaceList };
