// What wbd is and what it found beside it, asked each time wbd comes up,
// and again when you say something was installed.

import { createResource } from 'solid-js';
import { ask, status } from '../daemon';

const [health, { refetch }] = createResource(
  () => status() === 'ready',
  () => ask({ method: 'health' }, 'health'),
);

export { health };

/** Asks wbd again what it finds. */
export const recheck = () => refetch();
