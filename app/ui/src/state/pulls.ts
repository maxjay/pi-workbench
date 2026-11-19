// Your pull requests, as wbd last got them from dova, and the ones you were
// asked to review, with what you picked to see of those.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import type { Asked } from '../protocol/Asked';
import type { PullRow } from '../protocol/PullRow';
import { health } from './health';

/** Every folder the list was made from, and what was done with it: an empty list says why from this. */
const [asked, setAsked] = createSignal<Asked[]>([]);
/** Every project that was asked: the folders' own, and the others in their organizations with one of yours. */
const [searched, setSearched] = createSignal<string[]>([]);

// Nothing to ask until wbd is up and has found dova.
const ready = () => status() === 'ready' && Boolean(health()?.dova);

/** Your rows as dova first lists them, shown until the full answer comes. */
const [first, setFirst] = createSignal<PullRow[]>();

/** The ones you opened. Asked as soon as wbd is up, and shown without waiting for the others. */
const [pulls, { refetch }] = createResource(ready, async (_, known): Promise<PullRow[]> => {
  // The first time, the rows are also asked for without the reason under each: why a check
  // failed and which thread is open take a call for each row. If this fails, the full call says why.
  if (!known.value) {
    void ask({ method: 'pulls.list', params: { review: false, quick: true } }, 'pulls').then(
      (listed) => setFirst(listed.rows),
      () => undefined,
    );
  }
  const listed = await ask({ method: 'pulls.list', params: { review: false, quick: false } }, 'pulls');
  setAsked(listed.asked);
  setSearched(listed.searched);
  return listed.rows;
});

/** Your rows: the full answer once it is here, and before that the first one. */
const yours = () => pulls.latest ?? (pulls.error ? undefined : first());

/** The ones you or a team of yours were asked to review. Asked at the same time, in a list of their own. */
const [toReview, { refetch: refetchReview }] = createResource(ready, async () => {
  const listed = await ask({ method: 'pulls.list', params: { review: true, quick: false } }, 'pulls');
  return listed.rows;
});

export { asked, pulls, searched, toReview, yours };

/** Asks again. */
export const refreshPulls = () => Promise.all([refetch(), refetchReview()]);

/** A repo you chose, or with no repo the whole of a project. */
export type RepoPick = { project: string; repo: string | null };

/**
 * Whose pull requests to review show, and in which repos. A person is kept by their account in
 * the organization, with their name to show. With nothing chosen every row shows. With people
 * and repos both chosen, a row must match both, or with `either` one of them is enough. Kept in
 * the window's own storage.
 */
export type ReviewFilter = { people: { id: string; name: string }[]; repos: RepoPick[]; either: boolean };

function kept(): ReviewFilter {
  try {
    const saved = JSON.parse(localStorage.getItem('review-filter') ?? '{}') as Partial<ReviewFilter>;
    return { people: saved.people ?? [], repos: saved.repos ?? [], either: saved.either ?? false };
  } catch {
    return { people: [], repos: [], either: false };
  }
}

const [filter, setFilterNow] = createSignal<ReviewFilter>(kept());

export { filter };

export function setFilter(next: ReviewFilter) {
  localStorage.setItem('review-filter', JSON.stringify(next));
  setFilterNow(next);
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A repo as it is written: `project/repo`, or the project's name for the whole of it. */
export const repoName = (pick: RepoPick) =>
  pick.repo === null ? pick.project : `${pick.project}/${pick.repo}`;

/** Whether a row is in a repo you chose. */
export const inRepo = (pick: RepoPick, row: PullRow) =>
  same(pick.project, row.project) && (pick.repo === null || same(pick.repo, row.repo));

/** Whether a row was opened by a person you chose. By their account; by name for a row that has none. */
export const byPerson = (person: { id: string; name: string }, row: PullRow) =>
  row.authorId ? row.authorId === person.id : same(person.name, row.author ?? '');

/** Whether a row is among what you chose. */
export function shows(chosen: ReviewFilter, row: PullRow) {
  const person = chosen.people.some((one) => byPerson(one, row));
  const repo = chosen.repos.some((one) => inRepo(one, row));
  if (chosen.people.length === 0) return chosen.repos.length === 0 || repo;
  if (chosen.repos.length === 0) return person;
  return chosen.either ? person || repo : person && repo;
}
