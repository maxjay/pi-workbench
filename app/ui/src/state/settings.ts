// Pi's own files, as Settings shows them: AGENTS.md, the MCP servers in
// mcp.json, and the models and defaults in models.json and settings.json.
// wb-agent reads and writes them; this only keeps the latest it said.

import { createResource, createSignal } from 'solid-js';
import { ask, status } from '../agent';

export type Section = 'instructions' | 'mcp' | 'models';

export const sections: { id: Section; title: string; file: string }[] = [
  { id: 'instructions', title: 'Instructions', file: 'AGENTS.md' },
  { id: 'mcp', title: 'MCP servers', file: 'mcp.json' },
  { id: 'models', title: 'Models', file: 'models.json, settings.json' },
];

/** The thinking levels Pi knows (`pi --help`). */
export const thinkingLevels = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'];

const [section, setSection] = createSignal<Section>('instructions');

export { section, setSection };

const ready = () => status() === 'ready';

const [instructions, { refetch: reloadInstructions, mutate: setInstructions }] = createResource(ready, () =>
  ask({ method: 'config.read', params: { file: 'agents' } }, 'config'),
);
const [servers, { refetch: reloadServers }] = createResource(ready, async () => {
  const answer = await ask({ method: 'mcp.list' }, 'mcp');
  return answer.servers;
});
const [models, { refetch: reloadModels }] = createResource(ready, () =>
  ask({ method: 'models.list' }, 'models'),
);

export { instructions, models, servers };

/** Reads everything again, e.g. after `pi mcp add` in a terminal. */
export function reload() {
  void reloadInstructions();
  void reloadServers();
  void reloadModels();
}

export async function saveInstructions(text: string) {
  await ask({ method: 'config.write', params: { file: 'agents', text } }, 'done');
  const current = instructions();
  if (current) setInstructions({ ...current, exists: true, text });
}

export async function setServerEnabled(name: string, enabled: boolean) {
  await ask({ method: 'mcp.setEnabled', params: { name, enabled } }, 'done');
  await reloadServers();
}

export async function setDefaultModel(provider: string, model: string) {
  await ask({ method: 'models.setDefault', params: { provider, model } }, 'done');
  await reloadModels();
}

export async function setThinking(level: string) {
  await ask({ method: 'models.setThinking', params: { level } }, 'done');
  await reloadModels();
}
