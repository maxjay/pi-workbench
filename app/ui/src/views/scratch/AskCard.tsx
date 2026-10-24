// An extension waiting on an answer: pick one, yes or no, or type one.
// The run carries on once it's answered.

import { createSignal, For, Match, Show, Switch } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import type { Ask } from '../../protocol/Ask';
import type { AskReply } from '../../protocol/AskReply';
import { answer } from '../../state/sessions';

export function AskCard(props: { session: string; ask: Ask }) {
  const reply = createAction((value: AskReply) => answer(props.session, props.ask.id, value));
  const initial = () => (props.ask.kind.type === 'editor' ? props.ask.kind.prefill : '');
  const [text, setText] = createSignal(initial());
  const submit = () => void reply.run({ type: 'value', value: text() });

  return (
    <section class="ask-card" aria-label="Pi is asking">
      <p class="ask-title">{props.ask.title}</p>
      <Show when={props.ask.message}>
        <p class="ask-message">{props.ask.message}</p>
      </Show>
      <Switch>
        <Match when={props.ask.kind.type === 'select' && props.ask.kind}>
          {(kind) => (
            <div class="ask-options">
              <For each={kind().options}>
                {(option) => (
                  <button
                    type="button"
                    class="button"
                    disabled={reply.busy()}
                    onClick={() => void reply.run({ type: 'value', value: option })}
                  >
                    {option}
                  </button>
                )}
              </For>
            </div>
          )}
        </Match>
        <Match when={props.ask.kind.type === 'confirm'}>
          <div class="ask-options">
            <button
              type="button"
              class="button button-primary"
              disabled={reply.busy()}
              onClick={() => void reply.run({ type: 'confirmed', confirmed: true })}
            >
              Yes
            </button>
            <button
              type="button"
              class="button"
              disabled={reply.busy()}
              onClick={() => void reply.run({ type: 'confirmed', confirmed: false })}
            >
              No
            </button>
          </div>
        </Match>
        <Match when={props.ask.kind.type === 'input' || props.ask.kind.type === 'editor'}>
          <textarea
            class="ask-input"
            rows={props.ask.kind.type === 'editor' ? 8 : 1}
            placeholder={props.ask.kind.type === 'input' ? (props.ask.kind.placeholder ?? '') : ''}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && props.ask.kind.type === 'input') {
                e.preventDefault();
                submit();
              }
            }}
          />
          <div class="ask-options">
            <button type="button" class="button button-primary" disabled={reply.busy()} onClick={submit}>
              Send
            </button>
          </div>
        </Match>
      </Switch>
      <div class="ask-foot">
        <ErrorNote error={reply.error()} />
        <button
          type="button"
          class="button button-text"
          disabled={reply.busy()}
          onClick={() => void reply.run({ type: 'cancelled' })}
        >
          Dismiss
        </button>
      </div>
    </section>
  );
}
