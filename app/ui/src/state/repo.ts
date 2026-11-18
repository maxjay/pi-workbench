// What a folder holds, as last asked: its files, and where its git stands.

import { createStore } from 'solid-js/store';
import { ask } from '../daemon';
import type { GitStatus } from '../protocol/GitStatus';

export interface Repo {
  /** Paths relative to the folder, sorted. */
  files: string[];
  /** True when the folder has more files than are listed. */
  truncated: boolean;
  /** Null outside git. */
  git: GitStatus | null;
}

const [repos, setRepos] = createStore<Record<string, Repo>>({});

export { repos };

/** Folders being asked about now, so two askers share one answer. */
const asking = new Map<string, Promise<void>>();

/** Asks wbd for a folder's files and git status. */
export function refreshRepo(dir: string) {
  let going = asking.get(dir);
  if (!going) {
    going = Promise.all([
      ask({ method: 'files.list', params: { dir } }, 'files'),
      ask({ method: 'git.status', params: { dir } }, 'git'),
    ])
      .then(([files, git]) =>
        setRepos(dir, { files: files.files, truncated: files.truncated, git: git.status }),
      )
      .finally(() => asking.delete(dir));
    asking.set(dir, going);
  }
  return going;
}

/** A file as the last commit has it, or null. */
export async function original(path: string) {
  return (await ask({ method: 'git.original', params: { path } }, 'original')).text;
}
