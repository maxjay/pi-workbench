// Settings: the agents wbd found and which one a new conversation starts
// with, and the other programs the workbench works with.

import { createResource, For, Show } from 'solid-js';
import type { Agent } from '../protocol/Agent';
import { agentName, askOffer, defaultAgent, setDefaultAgent } from '../state/conversations';
import { health } from '../state/health';
import { active } from '../state/tabs';
import { setTheme, type Theme, theme } from '../state/theme';

export function Settings() {
  const version = (agent: Agent) => (agent === 'pi' ? health()?.pi : health()?.claude);
  return (
    <div class="home">
      <section class="section" aria-labelledby="agents">
        <div class="section-head">
          <h2 id="agents">Agents</h2>
        </div>
        <p class="dim">
          A new conversation starts with the default, on the model and effort you last picked for it. You can
          pick the other agent, model or effort for any conversation, where you write.
        </p>
        <div class="rows">
          <For each={['pi', 'claude'] as Agent[]}>
            {(agent) => (
              <div class="row setting-row">
                <span class="table-name">{agentName[agent]}</span>
                <span classList={{ dim: !version(agent) }}>
                  {version(agent) ?? 'not found in WSL'}
                  <Show when={version(agent)}>
                    <SignedIn agent={agent} />
                  </Show>
                </span>
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
              <dt>Theme</dt>
              <dd class="setting-theme">
                {/* System follows Windows. */}
                <For each={['system', 'light', 'dark'] as Theme[]}>
                  {(each) => (
                    <button
                      type="button"
                      class="quiet"
                      aria-pressed={theme() === each}
                      onClick={() => setTheme(each)}
                    >
                      {each}
                    </button>
                  )}
                </For>
              </dd>
            </dl>
          </section>
        )}
      </Show>
    </div>
  );
}

/** Whether an agent is signed in, asked from the agent each time Settings is shown. */
function SignedIn(props: { agent: Agent }) {
  const [offer] = createResource(
    () => active() === 'settings' && props.agent,
    (agent) => askOffer(agent),
  );
  return (
    <Show when={!offer.error} fallback={<span class="note-failed"> · {String(offer.error?.message)}</span>}>
      <Show when={offer.latest} fallback={<span class="dim"> · asking it</span>}>
        {(o) => (
          <Show
            when={o().signedIn}
            fallback={
              <span class="note-failed">
                {' '}
                · not signed in. Run {props.agent === 'pi' ? 'pi' : 'claude'} in a terminal to sign in.
              </span>
            }
          >
            <span class="dim"> · signed in</span>
          </Show>
        )}
      </Show>
    </Show>
  );
}
