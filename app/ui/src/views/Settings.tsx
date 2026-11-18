// Settings: the agents wbd found and which one a new conversation starts
// with, and the other programs the workbench works with.

import { For, Show } from 'solid-js';
import type { Agent } from '../protocol/Agent';
import { agentName, defaultAgent, setDefaultAgent } from '../state/conversations';
import { health } from '../state/health';

export function Settings() {
  const version = (agent: Agent) => (agent === 'pi' ? health()?.pi : health()?.claude);
  return (
    <div class="home">
      <section class="section" aria-labelledby="agents">
        <div class="section-head">
          <h2 id="agents">Agents</h2>
        </div>
        <p class="dim">
          A new conversation starts with the default. You can pick the other one for any conversation, where
          you write.
        </p>
        <div class="rows">
          <For each={['pi', 'claude'] as Agent[]}>
            {(agent) => (
              <div class="row setting-row">
                <span class="workspace-name">{agentName[agent]}</span>
                <span classList={{ dim: !version(agent) }}>{version(agent) ?? 'not found in WSL'}</span>
                <Show
                  when={defaultAgent() === agent}
                  fallback={
                    <button
                      type="button"
                      class="button"
                      disabled={!version(agent)}
                      onClick={() => setDefaultAgent(agent)}
                    >
                      Make default
                    </button>
                  }
                >
                  <span class="dim">Default</span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </section>

      <Show when={health()}>
        {(h) => (
          <section class="section" aria-labelledby="machine">
            <div class="section-head">
              <h2 id="machine">This machine</h2>
            </div>
            <dl class="facts">
              <dt>wbd</dt>
              <dd>{h().version}</dd>
              <dt>Runs in</dt>
              <dd>{h().distro ?? 'Linux, outside WSL'}</dd>
              <dt>dova</dt>
              <dd classList={{ dim: !h().dova }}>{h().dova ?? 'not found in WSL'}</dd>
            </dl>
          </section>
        )}
      </Show>
    </div>
  );
}
