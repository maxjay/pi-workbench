// A branch and its pull request, as dova reports them: the calls a
// codespace makes about its own branch, and to open a pull request from it.
// Reviewing one is in `review.ts`.

import { createSignal } from 'solid-js';
import { ask } from '../daemon';
import { refreshPulls } from './pulls';

/** A folder's branch and its pull request. With `pr`, that pull request whatever the branch. */
export const pullStatus = (dir: string, pr: number | null) =>
  ask({ method: 'pull.status', params: { dir, pr } }, 'pull');

/** Counts what was written to Azure DevOps from here, so lists know to ask again. */
const [written, setWritten] = createSignal(0);

export { written };

/** What a pull request from a folder's branch would hold. */
export const pullBefore = (dir: string) => ask({ method: 'pull.before', params: { dir } }, 'before');

/** Opens a pull request from a folder's branch. */
export async function createPull(dir: string, title: string, body: string, draft: boolean) {
  const created = await ask({ method: 'pull.create', params: { dir, title, body, draft } }, 'created');
  setWritten(written() + 1);
  void refreshPulls();
  return created;
}

/** Links work items to a folder's branch. */
export async function linkItems(dir: string, ids: number[]) {
  await ask({ method: 'pull.link', params: { dir, ids } }, 'done');
  setWritten(written() + 1);
}
