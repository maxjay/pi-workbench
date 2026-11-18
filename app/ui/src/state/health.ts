// What wbd is and what it found beside it, asked once each time wbd comes up.

import { createResource } from 'solid-js';
import { ask, status } from '../daemon';

const [health] = createResource(
  () => status() === 'ready',
  () => ask({ method: 'health' }, 'health'),
);

export { health };
