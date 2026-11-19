// Your pull requests, as wbd last got them from dova.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import type { Asked } from '../protocol/Asked';
import type { PullRow } from '../protocol/PullRow';
import { health } from './health';

/** Every folder the list was made from, and what was done with it: an empty list says why from this. */
const [asked, setAsked] = createSignal<Asked[]>([]);
/** Every project that was asked: the folders' own, and the others in their organizations with one of yours. */
const [searched, setSearched] = createSignal<string[]>([]);
/** The repos you care about, as you wrote them. Empty: all of them. Kept in the window's own storage. */
const [repos, setReposNow] = createSignal(localStorage.getItem('pull-repos') ?? '');

export function setRepos(text: string) {
  localStorage.setItem('pull-repos', text);
  setReposNow(text);
}

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

/** The ones you were asked to review. Asked at the same time, and added when they come. */
const [toReview, { refetch: refetchReview }] = createResource(ready, async () => {
  const listed = await ask({ method: 'pulls.list', params: { review: true, quick: false } }, 'pulls');
  setSearched((known) => [...new Set([...known, ...listed.searched])]);
  return listed.rows;
});

export { asked, pulls, repos, searched, toReview, yours };

/** Asks dova again. */
export const refreshPulls = () => Promise.all([refetch(), refetchReview()]);
