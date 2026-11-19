// Reviewing a pull request: the calls its tab makes, and what the page
// keeps for itself about one. Which files you have viewed and what the
// agent answered you are yours alone, so they live in the window's own
// storage and never go to Azure DevOps.

import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { ask } from '../daemon';
import type { ChangedFile } from '../protocol/ChangedFile';
import type { Place } from '../protocol/Place';
import type { PullTo } from '../protocol/PullTo';

/** Counts what was written to a pull request from here, so its tab knows to ask again. */
const [changed, setChanged] = createSignal(0);

export { changed };

const wrote = () => setChanged(changed() + 1);

export const openReview = (dir: string, pr: number) =>
  ask({ method: 'pull.open', params: { dir, pr } }, 'review');

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

/** Resolves a thread, or with `active` opens it again. */
export async function setThread(dir: string, pr: number, thread: number, status: 'resolved' | 'active') {
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
}

export const checks = async (dir: string, pr: number) =>
  (await ask({ method: 'pull.checks', params: { dir, pr } }, 'checks')).runs;

export const runSteps = (dir: string, run: number) =>
  ask({ method: 'pull.run', params: { dir, run } }, 'run');

export const stepLog = async (dir: string, run: number, log: number) =>
  (await ask({ method: 'pull.steplog', params: { dir, run, log } }, 'printed')).text;

export const rerun = (dir: string, run: number) =>
  ask({ method: 'pull.rerun', params: { dir, run } }, 'done');

/** The folder the agent works in for a pull request, and what it is told first. */
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

/** The files you marked as viewed: for each pull request, each path's mark when you did. */
const [viewed, setViewed] = createStore<Record<string, Record<string, string>>>(stored('viewed', {}));

/** A file is viewed while it is still the change you marked. A push that changes it unmarks it. */
export const isViewed = (pull: string, file: ChangedFile) => viewed[pull]?.[file.path] === file.mark;

export function markViewed(pull: string, files: ChangedFile[], yes: boolean) {
  for (const file of files) setViewed(pull, { [file.path]: yes ? file.mark : undefined });
  localStorage.setItem('viewed', JSON.stringify(viewed));
}

/** Something the agent answered you, shown where you asked. Only you see it. */
export interface Note {
  id: string;
  /** The lines asked about. Undefined for a note on a thread or a run. */
  place?: Place;
  /** The thread it drafts a reply for. */
  thread?: number;
  /** The run it explains. */
  run?: number;
  question: string;
  /** Undefined while the agent is still working on it. */
  answer?: string;
  failed?: string;
}

const [notes, setNotes] = createStore<Record<string, Note[]>>(stored('notes', {}));

export const notesOf = (pull: string) => notes[pull] ?? [];

function keep() {
  // One the agent never finished is not worth keeping past this window.
  const done = Object.fromEntries(
    Object.entries(notes).map(([pull, list]) => [pull, list.filter((note) => note.answer)]),
  );
  localStorage.setItem('notes', JSON.stringify(done));
}

export function addNote(pull: string, note: Note) {
  setNotes(pull, [...notesOf(pull), note]);
}

export function settleNote(pull: string, id: string, result: { answer?: string; failed?: string }) {
  setNotes(pull, (note) => note.id === id, result);
  keep();
}

export function dropNote(pull: string, id: string) {
  setNotes(
    pull,
    notesOf(pull).filter((note) => note.id !== id),
  );
  keep();
}
