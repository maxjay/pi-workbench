// The foot of a conversation: why Pi stopped, if it did; its question,
// which takes the place of the box you write in; what's waiting to be
// sent; the box; and a status line like Pi's.

import { For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { draftKey, setDraft } from '../../state/drafts';
import { reopen, unqueue } from '../../state/sessions';
import { AskCard } from './AskCard';
import { Composer } from './Composer';
import { usage } from './usage';

export function Foot(props: { session: Session }) {
  const state = () => props.session.state;
  const restart = createAction(async () => {
    const file = state().file;
    if (file) await reopen(file);
  });
  const takeBack = createAction(async (index: number, edit: boolean) => {
    const item = state().queued[index];
    await unqueue(props.session.id, index);
    if (edit && item) setDraft(draftKey(props.session), item.text);
  });

  return (
    <div class="conversation-foot">
      <Show when={state().stopped}>
        <div class="stopped" role="alert">
          <span>{state().stopped}</span>
          <Show when={state().file}>
            <button type="button" class="button" disabled={restart.busy()} onClick={() => void restart.run()}>
              Start Pi again
            </button>
          </Show>
        </div>
        <ErrorNote error={restart.error()} />
      </Show>
      <Show when={state().ask} keyed>
        {(ask) => <AskCard session={props.session.id} ask={ask} />}
      </Show>
      <Show when={state().queued.length > 0}>
        <ul class="queued" aria-label="Waiting to be sent">
          <For each={state().queued}>
            {(item, i) => (
              <li>
                <span class="queued-when">{item.later ? 'after this run' : 'next'}</span>
                <span class="queued-text">{item.text}</span>
                <button
                  type="button"
                  class="button button-text button-small"
                  disabled={takeBack.busy()}
                  onClick={() => void takeBack.run(i(), true)}
                >
                  Edit
                </button>
                <button
                  type="button"
                  class="icon-button icon-button-small"
                  aria-label="Don’t send it"
                  disabled={takeBack.busy()}
                  onClick={() => void takeBack.run(i(), false)}
                >
                  <Icon name="close" size={11} />
                </button>
              </li>
            )}
          </For>
        </ul>
        <ErrorNote error={takeBack.error()} />
      </Show>
      <Show when={!state().ask}>
        <Composer session={props.session} />
      </Show>
      <div class="status-line mono">
        <span title={state().cwd}>{state().cwd}</span>
        <For each={state().statuses}>{(line) => <span>{line.text}</span>}</For>
        <Show when={usage(state().usage)}>{(text) => <span>{text()}</span>}</Show>
      </div>
    </div>
  );
}
