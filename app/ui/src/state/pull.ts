// One pull request, as dova reports it: the calls a workspace makes about
// its branch's pull request, its threads and its runs.

import { createSignal } from 'solid-js';
import { ask } from '../daemon';
import { refreshPulls } from './pulls';

/** A folder's branch and its pull request. With `pr`, that pull request whatever the branch. */
export const pullStatus = (dir: string, pr: number | null) =>
  ask({ method: 'pull.status', params: { dir, pr } }, 'pull');

export const pullThread = (dir: string, pr: number, thread: number) =>
  ask({ method: 'pull.thread', params: { dir, pr, thread } }, 'thread');

export const pullLog = (dir: string, run: number) => ask({ method: 'pull.log', params: { dir, run } }, 'log');

/** Counts what was written to Azure DevOps from here, so lists know to ask again. */
const [written, setWritten] = createSignal(0);

export { written };

export async function reply(dir: string, pr: number, thread: number, text: string, resolve: boolean) {
  await ask({ method: 'pull.reply', params: { dir, pr, thread, text, resolve } }, 'done');
  setWritten(written() + 1);
}

export async function resolve(dir: string, pr: number, thread: number) {
  await ask({ method: 'pull.resolve', params: { dir, pr, thread } }, 'done');
  setWritten(written() + 1);
}

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
