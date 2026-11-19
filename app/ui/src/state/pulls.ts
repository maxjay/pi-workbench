// Your pull requests, as wbd last got them from dova.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../daemon';
import type { Asked } from '../protocol/Asked';
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

/** The ones you opened. Asked as soon as wbd is up, and shown without waiting for the others. */
const [pulls, { refetch }] = createResource(ready, async () => {
  const listed = await ask({ method: 'pulls.list', params: { review: false } }, 'pulls');
  setAsked(listed.asked);
  setSearched(listed.searched);
  return listed.rows;
});

/** The ones you were asked to review. Asked at the same time, and added when they come. */
const [toReview, { refetch: refetchReview }] = createResource(ready, async () => {
  const listed = await ask({ method: 'pulls.list', params: { review: true } }, 'pulls');
  setSearched((known) => [...new Set([...known, ...listed.searched])]);
  return listed.rows;
});

export { asked, pulls, repos, searched, toReview };

/** Asks dova again. */
export const refreshPulls = () => Promise.all([refetch(), refetchReview()]);
