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

/** Your rows as dova first lists them, shown until the full answer comes. */
const [first, setFirst] = createSignal<PullRow[]>();

/** The ones you opened. Asked as soon as wbd is up, and shown without waiting for the others. */
const [pulls, { refetch }] = createResource(ready, async (_, known): Promise<PullRow[]> => {
  // The first time, the rows are also asked for without the reason under each: why a check
  // failed and which thread is open take a call for each row. If this fails, the full call says why.
  if (!known.value) {
    void ask({ method: 'pulls.list', params: { quick: true } }, 'pulls').then(
      (listed) => setFirst(listed.rows),
      () => undefined,
    );
  }
  const listed = await ask({ method: 'pulls.list', params: { quick: false } }, 'pulls');
  setAsked(listed.asked);
  setSearched(listed.searched);
  return listed.rows;
});

/** Your rows: the full answer once it is here, and before that the first one. */
const yours = () => pulls.latest ?? (pulls.error ? undefined : first());

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
