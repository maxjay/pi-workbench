// Reviewing a pull request: the calls its tab makes, and what the page
// keeps for itself about one. Which files you have read, and what you and
// the agent said to each other about its lines and threads, are yours
// alone: they live in the window's own storage and never go to Azure DevOps.

import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { ask } from '../daemon';
import type { ChangedFile } from '../protocol/ChangedFile';
import type { Place } from '../protocol/Place';
import type { PullTo } from '../protocol/PullTo';
import { refreshPulls, refreshReview } from './pulls';

/** Counts what was written to a pull request from here, so its tab knows to ask again. */
const [changed, setChanged] = createSignal(0);

export { changed };

// Home's lists are asked again too: a vote, or a pull request that ended, shows there.
function wrote() {
  setChanged(changed() + 1);
  void refreshPulls();
  void refreshReview();
}

export const openReview = (dir: string, pr: number) =>
  ask({ method: 'pull.open', params: { dir, pr } }, 'review');

/** What changed in some of a pull request's files. */
export const loadFiles = async (dir: string, pr: number, paths: string[]) =>
  (await ask({ method: 'pull.files', params: { dir, pr, paths } }, 'changed')).files;

/** A changed file with every line of it, not only the changes. */
export const wholeFile = async (dir: string, pr: number, path: string) =>
  (await ask({ method: 'pull.diff', params: { dir, pr, path } }, 'fileDiff')).lines;

/** Starts a thread on lines of a file, or with no place on the whole pull request. */
export async function comment(dir: string, pr: number, text: string, place: Place | null) {
  await ask({ method: 'pull.comment', params: { dir, pr, text, place } }, 'done');
  wrote();
}

export async function reply(dir: string, pr: number, thread: number, text: string) {
  await ask({ method: 'pull.reply', params: { dir, pr, thread, text, resolve: false } }, 'done');
  wrote();
}

/** The states Azure DevOps has for a thread, by its own word for each. */
export const THREAD_STATES = [
  { status: 'active', label: 'Active' },
  { status: 'pending', label: 'Pending' },
  { status: 'fixed', label: 'Resolved' },
  { status: 'wontFix', label: "Won't fix" },
  { status: 'closed', label: 'Closed' },
] as const;

export async function setThread(dir: string, pr: number, thread: number, status: string) {
  await ask({ method: 'pull.resolve', params: { dir, pr, thread, status } }, 'done');
  wrote();
}

export async function vote(dir: string, pr: number, value: number) {
  await ask({ method: 'pull.vote', params: { dir, pr, vote: value } }, 'done');
  wrote();
}

export async function setPull(dir: string, pr: number, to: PullTo) {
  await ask({ method: 'pull.set', params: { dir, pr, to } }, 'done');
  wrote();
  // Azure DevOps takes a moment to complete one: asked again once it has. The checkout of one
  // that ended is removed then.
  setTimeout(() => {
    wrote();
    if (to === 'complete' || to === 'abandon') void ask({ method: 'pulls.sweep' }, 'done').catch(() => {});
  }, 3000);
}

/** Changes the title and the description. */
export async function editPull(dir: string, pr: number, title: string, description: string) {
  await ask({ method: 'pull.edit', params: { dir, pr, title, description } }, 'done');
  wrote();
}

/** The description as it was written, with its pictures as the links they are. */
export const describedAs = async (dir: string, pr: number) =>
  (await ask({ method: 'pull.description', params: { dir, pr } }, 'printed')).text;

/** Keeps a picture where dova can read it. Gives the path to write in a description. */
export const keepPicture = async (name: string, data: string) =>
  (await ask({ method: 'pull.picture', params: { name, data } }, 'printed')).text;

export const checks = async (dir: string, pr: number) =>
  (await ask({ method: 'pull.checks', params: { dir, pr } }, 'checks')).runs;

export const runSteps = (dir: string, run: number) =>
  ask({ method: 'pull.run', params: { dir, run } }, 'run');

export const stepLog = async (dir: string, run: number, log: number) =>
  (await ask({ method: 'pull.steplog', params: { dir, run, log } }, 'printed')).text;

export const rerun = (dir: string, run: number) =>
  ask({ method: 'pull.rerun', params: { dir, run } }, 'done');

/** Where a pull request's own folder is, or will be. Asking makes nothing. */
export const pullPlace = async (dir: string, pr: number) =>
  (await ask({ method: 'pull.place', params: { dir, pr } }, 'place')).cwd;

/** Makes a pull request's own folder if it is not there. Gives it, and what the agent is told first. */
export const session = (dir: string, pr: number) =>
  ask({ method: 'pull.session', params: { dir, pr } }, 'session');

/** Each open pull request's title, for its tab. */
const [titles, setTitles] = createStore<Record<string, string>>({});

export { titles };

export const setTitle = (tab: string, title: string) => setTitles(tab, title);

function stored<T>(key: string, empty: T): T {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null') ?? empty;
  } catch {
    return empty;
  }
}

/** The files you marked as read: for each pull request, each path's mark when you did. */
const [viewed, setViewed] = createStore<Record<string, Record<string, string>>>(stored('viewed', {}));

/** A file is read while it is still the change you marked. A push that changes it unmarks it. */
export const isViewed = (pull: string, file: ChangedFile) =>
  file.loaded && viewed[pull]?.[file.path] === file.mark;

export function markViewed(pull: string, files: ChangedFile[], yes: boolean) {
  // A file whose changes were not asked for yet has no mark to keep.
  for (const file of files) if (file.loaded) setViewed(pull, { [file.path]: yes ? file.mark : undefined });
  localStorage.setItem('viewed', JSON.stringify(viewed));
}

/** How the file list is cut down, in every pull request: what to leave out, and only what is unread. */
const [filter, setFilterNow] = createStore<{ exclude: string; unread: boolean }>(
  stored('review-filter', { exclude: '', unread: false }),
);

export { filter };

export function setFilter(next: Partial<{ exclude: string; unread: boolean }>) {
  setFilterNow(next);
  localStorage.setItem('review-filter', JSON.stringify(filter));
}

/** Old and new side by side, in every file, until you switch it back. */
const [split, setSplitNow] = createSignal(localStorage.getItem('review-split') === 'yes');

export { split };

export function setSplit(yes: boolean) {
  localStorage.setItem('review-split', yes ? 'yes' : 'no');
  setSplitNow(yes);
}

/** One thing you or the agent said, for you alone. */
export interface Said {
  id: string;
  who: 'you' | 'agent';
  /** When, in milliseconds since 1970. It places the message among a thread's comments. */
  at: number;
  /** Undefined while the agent is still working on it. */
  text?: string;
  failed?: string;
}

/**
 * What you and the agent said about one thing, for you alone: a thread, or
 * lines no thread is on yet. What the agent's review finds is one of these.
 */
export interface Talk {
  id: string;
  /** The thread it is about. */
  thread?: number;
  /** The lines it is about, when no thread is. */
  place?: Place;
  /** The agent's review made it. */
  finding?: boolean;
  /** You are done with it. */
  resolved?: boolean;
  said: Said[];
  /** A comment the agent wrote for you to post. Empty text while it is writing. */
  draft?: { text: string; writing: boolean };
  /** Whether Enter goes to the agent. Unset: to the agent while nothing is posted, to the thread once something is. */
  ask?: boolean;
}

/** A message the agent never finished is not worth showing as still coming. */
function settled(kept: Record<string, Talk[]>) {
  for (const list of Object.values(kept)) {
    for (const talk of list) {
      for (const said of talk.said) {
        if (said.text === undefined && !said.failed)
          said.failed = 'The window closed before the agent answered.';
      }
      if (talk.draft?.writing) talk.draft = undefined;
    }
  }
  // One you opened and said nothing in is not kept.
  for (const [pull, list] of Object.entries(kept)) {
    kept[pull] = list.filter((talk) => talk.thread !== undefined || talk.said.length > 0 || talk.draft);
  }
  return kept;
}

const [talks, setTalks] = createStore<Record<string, Talk[]>>(settled(stored('talks', {})));

const keep = () => localStorage.setItem('talks', JSON.stringify(talks));

export const talksOf = (pull: string) => talks[pull] ?? [];

export const threadTalk = (pull: string, thread: number) => talksOf(pull).find((t) => t.thread === thread);

export function addTalk(pull: string, talk: Omit<Talk, 'id'>): string {
  const id = crypto.randomUUID();
  setTalks(pull, [...talksOf(pull), { ...talk, id }]);
  keep();
  return id;
}

/** The talk about a thread, made if there is none yet. */
export function talkFor(pull: string, thread: number): string {
  return threadTalk(pull, thread)?.id ?? addTalk(pull, { thread, said: [] });
}

export function patchTalk(pull: string, id: string, patch: Partial<Talk>) {
  setTalks(pull, (talk) => talk.id === id, patch);
  keep();
}

export function dropTalk(pull: string, id: string) {
  setTalks(
    pull,
    talksOf(pull).filter((talk) => talk.id !== id),
  );
  keep();
}

/** Drops the talks that `gone` picks. */
export function dropTalks(pull: string, gone: (talk: Talk) => boolean) {
  setTalks(
    pull,
    talksOf(pull).filter((talk) => !gone(talk)),
  );
  keep();
}

export function say(pull: string, id: string, said: Omit<Said, 'id' | 'at'>): string {
  const one: Said = { ...said, id: crypto.randomUUID(), at: Date.now() };
  setTalks(
    pull,
    (talk) => talk.id === id,
    'said',
    (list) => [...list, one],
  );
  keep();
  return one.id;
}

export function settle(pull: string, id: string, said: string, result: { text?: string; failed?: string }) {
  setTalks(
    pull,
    (talk) => talk.id === id,
    'said',
    (one) => one.id === said,
    result,
  );
  keep();
}
