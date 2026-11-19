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

/**
 * What you picked to see of the ones to review. A repo is `project/repo`, and `project/*` is a
 * whole project. A row shows when its repo is picked and its author is picked; a list with
 * nothing picked lets every row through. Kept in the window's own storage.
 */
export type Picked = { repos: string[]; people: string[] };

function kept(): Picked {
  try {
    const saved = JSON.parse(localStorage.getItem('review-picked') ?? '{}') as Partial<Picked>;
    return { repos: saved.repos ?? [], people: saved.people ?? [] };
  } catch {
    return { repos: [], people: [] };
  }
}

const [picked, setPickedNow] = createSignal<Picked>(kept());

export { picked };

export function setPicked(next: Picked) {
  localStorage.setItem('review-picked', JSON.stringify(next));
  setPickedNow(next);
}

export const repoKey = (row: { project: string; repo: string }) => `${row.project}/${row.repo}`;

/** Whether a row is among what you picked. */
export function shows(pick: Picked, row: PullRow) {
  const repo =
    pick.repos.length === 0 || pick.repos.includes(repoKey(row)) || pick.repos.includes(`${row.project}/*`);
  const person = pick.people.length === 0 || pick.people.includes(row.author ?? '');
  return repo && person;
}
