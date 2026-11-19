// Your pull requests, as wbd last got them from dova, and the ones of other
// people that you chose to see: nothing of those is asked for until you choose.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import type { Asked } from '../protocol/Asked';
import type { PullRow } from '../protocol/PullRow';
import type { RepoPick } from '../protocol/RepoPick';
import { health } from './health';

/** Every folder the list was made from, and what was done with it: an empty list says why from this. */
const [asked, setAsked] = createSignal<Asked[]>([]);
/** Every project that was asked: the folders' own, and the others in their organizations with one of yours. */
const [searched, setSearched] = createSignal<string[]>([]);

// Nothing to ask until wbd is up and has found dova.
const ready = () => status() === 'ready' && Boolean(health()?.dova);

/**
 * Your rows as Azure DevOps' own list has them: one request. They are here first, and they say
 * which are open now, so one that was completed leaves as soon as this is asked again.
 */
const [first, setFirst] = createSignal<PullRow[]>();
/** When each list was last asked for. */
let askedAt = 0;
let reviewAt = 0;

/** The ones you opened, each with its check, its threads and what it waits on. These take longer. */
const [pulls, { refetch }] = createResource(ready, async (): Promise<PullRow[]> => {
  askedAt = Date.now();
  void ask({ method: 'pulls.list', params: { quick: true } }, 'pulls').then(
    (listed) => setFirst(listed.rows),
    // If this fails, the full call says why.
    () => setFirst(),
  );
  const listed = await ask({ method: 'pulls.list', params: { quick: false } }, 'pulls');
  setAsked(listed.asked);
  setSearched(listed.searched);
  return listed.rows;
});

const same = (row: PullRow) => `${row.project}/${row.repo}#${row.id}`.toLowerCase();

/** Your rows: which, from the list of one request. What is under each, from the full answer once it is here. */
const yours = () => {
  const full = pulls.latest;
  const rows = pulls.error ? undefined : first();
  if (!rows) return full;
  const known = new Map((full ?? []).map((row) => [same(row), row]));
  return rows.map((row) => known.get(same(row)) ?? row);
};

/**
 * Whose pull requests show under "To review", and in which repos. A person is kept by their
 * account in the organization, with their name to show. With people and repos both chosen, a
 * pull request must be by one of the people in one of the repos, or with `either` one of the two
 * is enough. Kept in the window's own storage.
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
/** True while the dialog that changes the filter is open: nothing is asked for until it closes. */
const [filtering, setFiltering] = createSignal(false);

export { filter, filtering, setFiltering };

export function setFilter(next: ReviewFilter) {
  localStorage.setItem('review-filter', JSON.stringify(next));
  setFilterNow(next);
}

/** A repo as it is written: `project/repo`, or the project's name for the whole of it. */
export const repoName = (pick: RepoPick) =>
  pick.repo === null ? pick.project : `${pick.project}/${pick.repo}`;

/**
 * The open pull requests of other people that you chose to see. Only what you chose is asked
 * for: one call for each repo, project or person. With nothing chosen, nothing is asked.
 */
const [toReview, { refetch: refetchReview }] = createResource(
  () => (ready() && !filtering() && filter().people.length + filter().repos.length > 0 ? filter() : false),
  async (chosen) => {
    reviewAt = Date.now();
    const params = {
      people: chosen.people.map((person) => person.id),
      repos: chosen.repos,
      both: !chosen.either,
    };
    const listed = await ask({ method: 'pulls.chosen', params }, 'pulls');
    return listed.rows;
  },
);

export { asked, pulls, searched, toReview, yours };

/** Asks dova for yours again. */
export const refreshPulls = () => refetch();

/** Asks for the ones you chose to see again. */
export const refreshReview = () => refetchReview();

/** Asks again for what was not asked for a moment ago: for when Home comes back on screen. */
export function refreshOld() {
  if (!pulls.loading && Date.now() - askedAt > 10_000) void refetch();
  if (!toReview.loading && Date.now() - reviewAt > 60_000) void refetchReview();
}
