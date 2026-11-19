// What a workspace's folder holds, as last asked: the folders read for the
// tree, and where its git stands.

import { createStore, reconcile, unwrap } from 'solid-js/store';
import { ask } from '../daemon';
import type { GitStatus } from '../protocol/GitStatus';

/** The names directly inside one folder, each list sorted. */
export interface Listing {
  folders: string[];
  files: string[];
}

export interface Repo {
  /**
   * Each folder that was read, by its path relative to the workspace. '' is
   * the workspace's own folder. A folder that could not be read is not here.
   */
  folders: Record<string, Listing>;
  /** Null outside git. */
  git: GitStatus | null;
}

const [repos, setRepos] = createStore<Record<string, Repo>>({});

export { repos };

/** Folders being asked about now, so two askers share one answer. */
const asking = new Map<string, Promise<void>>();

/** Asks wbd what one folder of a workspace holds. Undefined when it cannot be read. */
const read = (dir: string, path: string) =>
  ask({ method: 'folder.read', params: { dir: path ? `${dir}/${path}` : dir } }, 'entries').then(
    ({ folders, files }): Listing => ({ folders, files }),
    () => undefined,
  );

/**
 * The folders kept for a workspace with what was just read put in. A folder
 * that could not be read is dropped. One that was not asked about stays as
 * it was, so an answer never takes away a folder opened after the question.
 */
function withRead(dir: string, paths: string[], listings: (Listing | undefined)[]) {
  const folders = { ...unwrap(repos[dir]?.folders) };
  paths.forEach((path, i) => {
    const listing = listings[i];
    if (listing) folders[path] = listing;
    else delete folders[path];
  });
  return folders;
}

/**
 * Writes only what differs, so what did not change is not drawn again.
 * No key: a folder may be called `id`, which is the key `reconcile` looks for.
 */
const changes = <T>(fresh: T) => reconcile(fresh, { key: null, merge: true });

/**
 * Asks wbd for a workspace's own folder, each folder open in its tree, and
 * its git status.
 */
export function refreshRepo(dir: string, open: string[]) {
  let going = asking.get(dir);
  if (!going) {
    const paths = ['', ...open];
    going = Promise.all([
      Promise.all(paths.map((path) => read(dir, path))),
      ask({ method: 'git.status', params: { dir } }, 'git'),
    ])
      .then(([listings, git]) =>
        setRepos(dir, changes({ folders: withRead(dir, paths, listings), git: git.status })),
      )
      .finally(() => asking.delete(dir));
    asking.set(dir, going);
  }
  return going;
}

/** Reads one folder of a workspace now, for when it is opened in the tree. */
export async function readFolder(dir: string, path: string) {
  const listing = await read(dir, path);
  if (repos[dir]) setRepos(dir, 'folders', changes(withRead(dir, [path], [listing])));
}

/** Every file in a workspace, relative to it, to find one by name. */
export const allFiles = (dir: string) => ask({ method: 'files.list', params: { dir } }, 'files');

/** A file as the last commit has it, or null. */
export async function original(path: string) {
  return (await ask({ method: 'git.original', params: { path } }, 'original')).text;
}
