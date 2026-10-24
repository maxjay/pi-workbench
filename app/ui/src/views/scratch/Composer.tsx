// Where you talk to Pi, as in a terminal: Enter sends, and while Pi works
// it steers the run; Alt+Enter waits for the run to finish instead; Escape
// stops it. The model and thinking level for this conversation sit below.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { abort, send, setModel, setThinking } from '../../state/sessions';
import { models, thinkingLevels } from '../../state/settings';

export function Composer(props: { session: Session }) {
  const [text, setText] = createSignal('');
  const busy = () => props.session.state.busy;
  const stopped = () => props.session.state.stopped !== null;
  const sending = createAction(async (later: boolean) => {
    const message = text().trim();
    if (!message) return;
    await send(props.session.id, message, later);
    setText('');
  });
  const stopping = createAction(() => abort(props.session.id));
  const change = createAction(async (pick: () => Promise<unknown>) => pick());

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void sending.run(e.altKey);
    } else if (e.key === 'Escape' && busy()) {
      e.preventDefault();
      e.stopPropagation();
      void stopping.run();
    }
  };

  return (
    <div class="composer">
      <Show when={props.session.state.queued.length > 0}>
        <ul class="queued" aria-label="Waiting to be sent">
          <For each={props.session.state.queued}>
            {(item) => (
              <li>
                <span class="queued-when">{item.later ? 'after this run' : 'next'}</span>
                {item.text}
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="composer-box">
        <textarea
          class="composer-input"
          rows={2}
          aria-label="Message Pi"
          placeholder={busy() ? 'Steer Pi, or Alt+Enter to queue for after this run…' : 'Message Pi…'}
          value={text()}
          disabled={stopped()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={onKey}
        />
        <div class="composer-bar">
          <select
            class="chip"
            aria-label="Model for this conversation"
            value={props.session.state.model ?? ''}
            disabled={stopped() || change.busy()}
            onChange={(e) => {
              const [provider, ...rest] = e.currentTarget.value.split('/');
              void change.run(() => setModel(props.session.id, provider, rest.join('/')));
            }}
          >
            <Show when={!props.session.state.model}>
              <option value="">No model</option>
            </Show>
            <For each={models.error ? [] : (models.latest?.providers ?? [])}>
              {(provider) => (
                <optgroup label={provider.name ?? provider.id}>
                  <For each={provider.models}>
                    {(model) => (
                      <option value={`${provider.id}/${model.id}`}>{model.name ?? model.id}</option>
                    )}
                  </For>
                </optgroup>
              )}
            </For>
          </select>
          <select
            class="chip"
            aria-label="Thinking"
            value={props.session.state.thinking ?? 'medium'}
            disabled={stopped() || change.busy()}
            onChange={(e) => {
              const level = e.currentTarget.value;
              void change.run(() => setThinking(props.session.id, level));
            }}
          >
            <For each={thinkingLevels}>{(level) => <option value={level}>thinking: {level}</option>}</For>
          </select>
          <span class="composer-hint">
            {busy()
              ? 'Enter steers · Alt+Enter queues · Esc stops'
              : 'Enter sends · Shift+Enter for a new line'}
          </span>
          <Show
            when={busy()}
            fallback={
              <button
                type="button"
                class="send"
                aria-label="Send"
                disabled={stopped() || sending.busy() || !text().trim()}
                onClick={() => void sending.run(false)}
              >
                <Icon name="send" size={16} />
              </button>
            }
          >
            <button
              type="button"
              class="send send-stop"
              aria-label="Stop"
              disabled={stopping.busy()}
              onClick={() => void stopping.run()}
            >
              <Icon name="stop" size={14} />
            </button>
          </Show>
        </div>
      </div>
      <ErrorNote error={sending.error() ?? stopping.error() ?? change.error()} />
    </div>
  );
}
