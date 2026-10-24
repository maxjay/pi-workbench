// models.json and the defaults in settings.json: which models the local
// server offers, and which one (and how much thinking) sessions start with.

import { For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Empty, ErrorNote } from '../../components/Note';
import type { Model } from '../../protocol/Model';
import { thinkingLevels as levels, models, setDefaultModel, setThinking } from '../../state/settings';

function contextSize(tokens: number | null) {
  if (!tokens) return undefined;
  return tokens >= 1_000_000 ? `${tokens / 1_000_000}M context` : `${Math.round(tokens / 1000)}k context`;
}

export function Models() {
  const pick = createAction(setDefaultModel);
  const think = createAction(setThinking);
  const current = () => (models.error ? undefined : models.latest);
  const isDefault = (provider: string, model: Model) =>
    current()?.defaults.provider === provider && current()?.defaults.model === model.id;
  const thinking = () => current()?.defaults.thinking ?? 'medium';
  const choices = () => (levels.includes(thinking()) ? levels : [...levels, thinking()]);

  return (
    <section class="settings-section">
      <header class="section-head">
        <div>
          <h1>Models</h1>
          <p class="section-sub">What your model server offers. New sessions start with the default.</p>
        </div>
        <label class="field">
          <span>Thinking</span>
          <select
            class="select"
            value={thinking()}
            disabled={!current() || think.busy()}
            onChange={(e) => void think.run(e.currentTarget.value)}
          >
            <For each={choices()}>{(level) => <option value={level}>{level}</option>}</For>
          </select>
        </label>
      </header>
      <ErrorNote error={models.error ?? pick.error() ?? think.error()} />
      <Show when={current()?.providers.length === 0}>
        <Empty>No providers in models.json yet.</Empty>
      </Show>
      <For each={current()?.providers ?? []}>
        {(provider) => (
          <div class="group">
            <div class="group-head">
              <h2>{provider.name ?? provider.id}</h2>
              <span class="mono">{provider.baseUrl}</span>
            </div>
            <ul class="rows">
              <For each={provider.models}>
                {(model) => (
                  <li class="row" aria-current={isDefault(provider.id, model) ? 'true' : undefined}>
                    <div class="row-main">
                      <div class="row-title">
                        <span>{model.name ?? model.id}</span>
                        <Show when={model.reasoning}>
                          <span class="tag">reasoning</span>
                        </Show>
                      </div>
                      <p class="row-detail">
                        <span class="mono">{model.id}</span>
                        <Show when={contextSize(model.contextWindow)}>{(size) => <> · {size()}</>}</Show>
                      </p>
                    </div>
                    <Show
                      when={!isDefault(provider.id, model)}
                      fallback={<span class="tag tag-accent">default</span>}
                    >
                      <button
                        type="button"
                        class="button"
                        disabled={pick.busy()}
                        onClick={() => void pick.run(provider.id, model.id)}
                      >
                        Make default
                      </button>
                    </Show>
                  </li>
                )}
              </For>
              <Show when={provider.models.length === 0}>
                <Empty>This provider lists no models.</Empty>
              </Show>
            </ul>
          </div>
        )}
      </For>
    </section>
  );
}
