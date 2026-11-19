// Settings: the agents wbd found and which one a new conversation starts
// with, the two folders the workbench works in, and the other programs it
// works with.

import { createResource, For, Show } from 'solid-js';
import { createAction } from '../components/action';
import { ask } from '../daemon';
import type { Agent } from '../protocol/Agent';
import type { Settings } from '../protocol/Settings';
import { agentName, askOffer, defaultAgent, setDefaultAgent } from '../state/conversations';
import { health } from '../state/health';
import { active } from '../state/tabs';
import { setTheme, type Theme, theme } from '../state/theme';

export function SettingsPage() {
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

      <Folders />

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

/**
 * The folder your clones are in, and the workbench's own. A pull request or a ticket opens
 * from any clone in the first. A pull request's checkout for the agent is made in the second,
 * and removed when the pull request ends.
 */
function Folders() {
  const [settings, { mutate }] = createResource(
    () => active() === 'settings',
    async () => (await ask({ method: 'settings.get' }, 'settings')) as Settings,
  );
  const saving = createAction(async (next: { repos: string; own: string }) => {
    mutate((await ask({ method: 'settings.set', params: next }, 'settings')) as Settings);
  });
  /** Saves a folder when what is written in its field is not what is kept. */
  const save = (which: 'repos' | 'own', written: string) => {
    const now = settings.latest;
    if (!now || written.trim() === now[which]) return;
    void saving.run({ repos: now.repos, own: now.own, [which]: written });
  };
  const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const name = (path: string) => path.slice(path.lastIndexOf('/') + 1);
  const field = (which: 'repos' | 'own', label: string) => (
    <input
      class="field setting-folder"
      aria-label={label}
      value={settings.latest?.[which] ?? ''}
      onChange={(e) => save(which, e.currentTarget.value)}
    />
  );

  return (
    <section class="section" aria-labelledby="folders">
      <div class="section-head">
        <h2 id="folders">Folders</h2>
      </div>
      <p class="dim">
        A pull request or a ticket opens from any clone in your repos folder, whether or not it is open as a
        codespace. The agent works on a pull request in a checkout of its own, in the workbench's folder. It
        is made from your clone when you first ask the agent something, and removed when the pull request is
        completed or abandoned.
      </p>
      <div class="rows">
        <div class="row setting-row">
          <span class="table-name">Your repos</span>
          {field('repos', 'The folder your clones are in')}
          <span class="dim">
            <Show when={settings.latest}>{(s) => count(s().clones, 'clone', 'clones')}</Show>
          </span>
        </div>
        <div class="row setting-row">
          <span class="table-name">The workbench's own</span>
          {field('own', 'The folder the workbench keeps its own things in')}
          <span class="dim">
            <Show when={settings.latest}>{(s) => count(s().checkouts.length, 'checkout', 'checkouts')}</Show>
          </span>
        </div>
      </div>
      <Show when={saving.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
      <Show when={settings.latest?.kept.length}>
        <p class="dim">
          Kept though their pull request ended, because they hold work that was not committed or not pushed:{' '}
          {settings.latest?.kept.map(name).join(', ')}.
        </p>
      </Show>
    </section>
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
