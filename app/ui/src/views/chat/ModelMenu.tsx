// The model and thinking level for one conversation, from the chip under
// the box you write in. The models are the ones Pi can use, narrowed to
// the ones you cycle through (Settings › Models) when you've picked some;
// the levels are the ones the model takes.

import { createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import type { Available } from '../../protocol/Available';
import type { Session } from '../../protocol/Session';
import { availableModels, setModel, setThinking } from '../../state/sessions';
import {
  models,
  setDefaultModel,
  setThinking as setDefaultThinking,
  thinkingLevels,
} from '../../state/settings';
import { tokens } from './usage';

/** The last list Pi gave, so a saved conversation's menu has one without starting Pi. */
const [known, setKnown] = createSignal<Available[]>([]);

/** Learns the models' names from a live session, once, for the chip. */
export async function learnModels(id: string) {
  if (known().length === 0) setKnown(await availableModels(id));
}

/** A model's name, from its `provider/id`. */
export function modelLabel(model: string | null) {
  if (!model) return 'No model';
  const found = known().find((m) => `${m.provider}/${m.id}` === model);
  return found?.name ?? model.slice(model.indexOf('/') + 1);
}

const levelLabel = (level: string) =>
  level === 'xhigh' ? 'X-high' : level[0].toUpperCase() + level.slice(1);

/** Whether a model matches one of Pi's `enabledModels` patterns: an id or `provider/id`, globs allowed. */
function matches(model: Available, pattern: string) {
  const bare = pattern.replace(/:[a-z]+$/i, '').toLowerCase();
  const target = (bare.includes('/') ? `${model.provider}/${model.id}` : model.id).toLowerCase();
  if (!bare.includes('*') && !bare.includes('?')) return target === bare;
  const glob = bare
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*/g, '.*')
    .replace(/\?/g, '.');
  return new RegExp(`^${glob}$`).test(target);
}

export function ModelMenu(props: { session: Session; onClose: () => void }) {
  const state = () => props.session.state;
  const [list] = createResource(
    () => props.session.id || undefined,
    async (id) => {
      const all = await availableModels(id);
      setKnown(all);
      return all;
    },
  );
  const shown = () => {
    const all = list.latest ?? known();
    const patterns = models.latest?.enabled ?? [];
    const picked = patterns.length > 0 ? all.filter((m) => patterns.some((p) => matches(m, p))) : all;
    // The one in use stays listed even when it isn't one you cycle through.
    const inUse = all.find((m) => `${m.provider}/${m.id}` === state().model);
    return inUse && !picked.includes(inUse) ? [inUse, ...picked] : picked;
  };
  const groups = () => {
    const byProvider = new Map<string, Available[]>();
    for (const model of shown())
      byProvider.set(model.provider, [...(byProvider.get(model.provider) ?? []), model]);
    // The provider in use first.
    const inUse = state().model?.split('/')[0];
    return [...byProvider.entries()].sort(([a], [b]) => Number(b === inUse) - Number(a === inUse));
  };
  const levels = () => (state().levels.length > 0 ? state().levels : thinkingLevels);
  const defaults = () => models.latest?.defaults;
  /** The model and level match the defaults; a level counts only for a model that thinks. */
  const isDefault = () => {
    const d = defaults();
    if (!d?.model || state().model !== `${d.provider}/${d.model}`) return false;
    return levels().length <= 1 || !d.thinking || state().thinking === d.thinking;
  };
  const change = createAction(async (work: () => Promise<unknown>) => work());

  let menu!: HTMLDivElement;
  onMount(() => {
    const away = (e: MouseEvent) => {
      if (!menu.parentElement?.contains(e.target as Node)) props.onClose();
    };
    const onEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') props.onClose();
    };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', onEscape);
    onCleanup(() => {
      document.removeEventListener('mousedown', away);
      document.removeEventListener('keydown', onEscape);
    });
  });

  return (
    <div class="model-menu" role="dialog" aria-label="Model for this conversation" ref={menu}>
      <div class="model-menu-head">
        <span>Model for this conversation</span>
        <Show when={state().usage?.context}>
          {(context) => <span class="count">{tokens(context())} tokens so far</span>}
        </Show>
      </div>
      <ErrorNote error={list.error} />
      <div class="model-list">
        <Show when={!list.loading || known().length > 0} fallback={<p class="model-empty">Asking Pi…</p>}>
          <For each={groups()} fallback={<p class="model-empty">Pi has no models it can use yet.</p>}>
            {([provider, group]) => (
              <>
                <p class="model-group">{provider}</p>
                <For each={group}>
                  {(model) => {
                    const on = () => state().model === `${model.provider}/${model.id}`;
                    return (
                      <button
                        type="button"
                        role="menuitemradio"
                        class="model-row"
                        aria-checked={on()}
                        disabled={change.busy()}
                        onClick={() => {
                          if (on()) return props.onClose();
                          void change.run(async () => {
                            await setModel(props.session, model.provider, model.id);
                            props.onClose();
                          });
                        }}
                      >
                        <span class="model-check" aria-hidden="true">
                          {on() ? '✓' : ''}
                        </span>
                        <span class="model-name">{model.name}</span>
                        <Show when={model.contextWindow}>
                          {(window) => <span class="count">{tokens(window())} context</span>}
                        </Show>
                      </button>
                    );
                  }}
                </For>
              </>
            )}
          </For>
        </Show>
      </div>
      <div class="model-menu-line" />
      <div class="model-thinking">
        <span>Thinking</span>
        <Show
          when={levels().length > 1}
          fallback={<span class="count">This model has no thinking levels</span>}
        >
          <fieldset class="segmented">
            <legend class="visually-hidden">Thinking</legend>
            <For each={levels()}>
              {(level) => (
                <button
                  type="button"
                  aria-pressed={state().thinking === level}
                  disabled={change.busy()}
                  onClick={() => void change.run(() => setThinking(props.session, level))}
                >
                  {levelLabel(level)}
                </button>
              )}
            </For>
          </fieldset>
        </Show>
      </div>
      <div class="model-default">
        <span>
          {isDefault()
            ? 'This is the default for new conversations'
            : `New conversations use ${defaults()?.model ? modelLabel(`${defaults()?.provider}/${defaults()?.model}`) : 'Pi’s default'}${defaults()?.thinking ? ` · ${defaults()?.thinking}` : ''}`}
        </span>
        <Show when={!isDefault() && state().model}>
          <button
            type="button"
            class="button button-text button-small button-accent"
            disabled={change.busy()}
            onClick={() =>
              void change.run(async () => {
                const [provider, ...rest] = (state().model ?? '').split('/');
                await setDefaultModel(provider, rest.join('/'));
                if (state().thinking) await setDefaultThinking(state().thinking as string);
              })
            }
          >
            Make it the default
          </button>
        </Show>
      </div>
      <ErrorNote error={change.error()} />
    </div>
  );
}
