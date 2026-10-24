// Who wb-agent is and what it found, asked for once it says it's ready.

import { createResource } from 'solid-js';
import { ask, status } from '../agent';

const [health] = createResource(
  () => status() === 'ready',
  () => ask({ method: 'health' }, 'health'),
);

export { health };
