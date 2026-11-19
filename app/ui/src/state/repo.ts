// What a codespace's folder holds, as last asked: the folders read for the
// tree, and where its git stands. A repo inside a codespace is kept here too,
// by its own folder, for where its git stands.

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
   * Each folder that was read, by its path relative to the codespace. '' is
   * the codespace's own folder. A folder that could not be read is not here.
   */
  folders: Record<string, Listing>;
  /**
   * Null outside git. Not there until git answered: in a large repo that takes a while, and
   * the tree does not wait for it.
   */
  git?: GitStatus | null;
}

const [repos, setRepos] = createStore<Record<string, Repo>>({});

export { repos };

/** Folders being asked about now, so two askers share one answer. */
const asking = new Map<string, Promise<void>>();

/** Asks wbd what one folder of a codespace holds. Undefined when it cannot be read. */
const read = (dir: string, path: string) =>
  ask({ method: 'folder.read', params: { dir: path ? `${dir}/${path}` : dir } }, 'entries').then(
    ({ folders, files }): Listing => ({ folders, files }),
    () => undefined,
  );

/**
 * The folders kept for a codespace with what was just read put in. A folder
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

/** For each codespace, whether git is being asked now, when it last answered and how long that took. */
const gitting = new Map<string, { busy: boolean; done: number; took: number }>();

/**
 * Asks git where a folder stands, apart from its tree. It is not asked again while it is
 * being asked, nor sooner after an answer than the answer took: where `git status` takes ten
 * seconds, it does not run all the time.
 */
export async function refreshGit(dir: string) {
  const last = gitting.get(dir);
  if (last && (last.busy || Date.now() - last.done < last.took)) return;
  const started = Date.now();
  gitting.set(dir, { busy: true, done: 0, took: 0 });
  try {
    const git = await ask({ method: 'git.status', params: { dir } }, 'git');
    // The tree is first: git's answer is put with a folder that was read.
    await asking.get(dir);
    if (repos[dir]) setRepos(dir, 'git', changes(git.status));
    else setRepos(dir, { folders: {}, git: git.status });
  } catch {
    // wbd went away. The next time is asked as any other.
  } finally {
    gitting.set(dir, { busy: false, done: Date.now(), took: Date.now() - started });
  }
}

/**
 * Asks wbd for a codespace's own folder and each folder open in its tree. Its git status is
 * asked for too, but the tree does not wait for it.
 */
export function refreshRepo(dir: string, open: string[]) {
  let going = asking.get(dir);
  if (!going) {
    const paths = ['', ...open];
    going = Promise.all(paths.map((path) => read(dir, path)))
      .then((listings) => {
        const folders = withRead(dir, paths, listings);
        if (repos[dir]) setRepos(dir, 'folders', changes(folders));
        else setRepos(dir, { folders });
      })
      .finally(() => asking.delete(dir));
    asking.set(dir, going);
  }
  void refreshGit(dir);
  return going;
}

/** Reads one folder of a codespace now, for when it is opened in the tree. */
export async function readFolder(dir: string, path: string) {
  const listing = await read(dir, path);
  if (repos[dir]) setRepos(dir, 'folders', changes(withRead(dir, [path], [listing])));
}

/** Every file in a codespace, relative to it, to find one by name. */
export const allFiles = (dir: string) => ask({ method: 'files.list', params: { dir } }, 'files');

/** A file as the last commit has it, or null. */
export async function original(path: string) {
  return (await ask({ method: 'git.original', params: { path } }, 'original')).text;
}
