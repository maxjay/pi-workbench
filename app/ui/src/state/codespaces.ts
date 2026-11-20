// The codespaces, as wbd lists them.

import { createResource } from 'solid-js';
import { ask, onEvent, status } from '../daemon';
import { close, open } from './tabs';

const [codespaces, { refetch }] = createResource(
  () => status() === 'ready',
  async () => (await ask({ method: 'codespaces.list' }, 'codespaces')).list,
);

export { codespaces };

// wbd says when one was named or got a repo, which an agent does without the page asking.
onEvent((event) => {
  if (event.event === 'codespaces') void refetch();
});

export const codespaceById = (id: string) => codespaces()?.find((view) => view.codespace.id === id);

/**
 * The folder a pull request or a ticket is asked from. Its tab names it by a codespace's id, or
 * for a clone in your repos folder that is not a codespace by the clone's path.
 */
export const dirOf = (key: string) => (/^[/@]/.test(key) ? key : codespaceById(key)?.codespace.path);

/** Opens the codespace a pull request or a ticket was asked from. A clone becomes one first. */
export const openPlace = (key: string) => (key.startsWith('/') ? openFolder(key) : open(`cs:${key}`));

/** Opens a folder as a codespace, and its tab. */
export async function openFolder(path: string) {
  const view = await ask({ method: 'codespaces.open', params: { path } }, 'codespace');
  await refetch();
  open(`cs:${view.codespace.id}`);
}

/** Takes a codespace off the list and closes its tab. The folder is not touched. */
export async function forget(id: string) {
  await ask({ method: 'codespaces.forget', params: { id } }, 'done');
  close(`cs:${id}`);
  await refetch();
}

/** Asks again, for where each codespace's git stands now. */
export const refreshCodespaces = () => void refetch();
