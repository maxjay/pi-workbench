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

const [pulls, { refetch }] = createResource(
  // Nothing to ask until wbd is up and has found dova.
  () => status() === 'ready' && Boolean(health()?.dova),
  async () => {
    const listed = await ask({ method: 'pulls.list' }, 'pulls');
    setAsked(listed.asked);
    setSearched(listed.searched);
    return listed.rows;
  },
);

export { asked, pulls, repos, searched };

/** Asks dova again. */
export const refreshPulls = () => refetch();
