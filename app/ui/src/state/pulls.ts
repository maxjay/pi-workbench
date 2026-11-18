// Your pull requests, as wbd last got them from dova.

import { createResource } from 'solid-js';
import { ask, status } from '../daemon';
import { health } from './health';

const [pulls, { refetch }] = createResource(
  // Nothing to ask until wbd is up and has found dova.
  () => status() === 'ready' && Boolean(health()?.dova),
  async () => (await ask({ method: 'pulls.list' }, 'pulls')).rows,
);

export { pulls };

/** Asks dova again. */
export const refreshPulls = () => refetch();
