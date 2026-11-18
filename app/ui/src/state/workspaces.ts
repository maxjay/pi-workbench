// The workspaces, as wbd lists them.

import { createResource } from 'solid-js';
import { ask, status } from '../daemon';
import { close, open } from './tabs';

const [workspaces, { refetch }] = createResource(
  () => status() === 'ready',
  async () => (await ask({ method: 'workspaces.list' }, 'workspaces')).list,
);

export { workspaces };

export const workspaceById = (id: string) => workspaces()?.find((view) => view.workspace.id === id);

/** Opens a folder as a workspace, and its tab. */
export async function openFolder(path: string) {
  const view = await ask({ method: 'workspaces.open', params: { path } }, 'workspace');
  await refetch();
  open(`ws:${view.workspace.id}`);
}

/** Takes a workspace off the list and closes its tab. The folder is not touched. */
export async function forget(id: string) {
  await ask({ method: 'workspaces.forget', params: { id } }, 'done');
  close(`ws:${id}`);
  await refetch();
}

/** Asks again, for where each workspace's git stands now. */
export const refreshWorkspaces = () => void refetch();
