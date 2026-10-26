// A codespace's files and where its git stands, asked for while its tab is
// open: now and then, and when Pi or you may have changed something.

import { createStore } from 'solid-js/store';
import { ask } from '../agent';
import type { GitStatus } from '../protocol/GitStatus';
import { listFiles } from './sessions';

export interface Repo {
  /** Files, relative to the codespace, sorted. */
  files: string[];
  truncated: boolean;
  /** None outside git. */
  git: GitStatus | null;
}

const [repos, setRepos] = createStore<Record<string, Repo>>({});

export { repos };

const asking = new Map<string, Promise<void>>();

/** Asks again for a folder's files and git, unless that's already under way. */
export function refreshRepo(dir: string) {
  const going = asking.get(dir);
  if (going) return going;
  const done = (async () => {
    try {
      const [files, git] = await Promise.all([
        listFiles(dir),
        ask({ method: 'git.status', params: { dir } }, 'git'),
      ]);
      setRepos(dir, { files: files.paths, truncated: files.truncated, git: git.status });
    } finally {
      asking.delete(dir);
    }
  })();
  asking.set(dir, done);
  return done;
}

/** A file as the last commit has it; none when the commit doesn't have it. */
export async function original(path: string) {
  return (await ask({ method: 'git.original', params: { path } }, 'original')).text;
}
