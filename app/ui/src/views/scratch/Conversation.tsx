// A scratchpad's conversation is the page, as with Pi in a terminal: what
// you asked, what Pi thought, ran and said. It follows the newest line
// unless you've scrolled up to read.

import { For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { closeSession, reopen } from '../../state/sessions';
import { AskCard } from './AskCard';
import { Composer } from './Composer';
import { EntryView } from './EntryView';

const folder = (cwd: string) => cwd.split('/').pop() ?? cwd;

export function Conversation(props: { session: Session; filesOpen: boolean; onFiles: () => void }) {
  const state = () => props.session.state;
  const close = createAction(() => closeSession(props.session.id));
  const restart = createAction(async () => {
    const file = state().file;
    if (file) await reopen(file);
  });

  let log!: HTMLDivElement;
  let content!: HTMLDivElement;
  let following = true;
  onMount(() => {
    const follow = new ResizeObserver(() => {
      if (following) log.scrollTop = log.scrollHeight;
    });
    follow.observe(content);
    onCleanup(() => follow.disconnect());
  });

  const doing = () => {
    if (state().stopped) return 'Stopped';
    if (state().ask) return 'Waiting for you';
    return state().busy ? 'Working…' : 'Idle';
  };

  return (
    <main class="conversation">
      <header class="conversation-head">
        <span class="pad-dot" data-state={state().stopped ? 'stopped' : state().busy ? 'busy' : 'live'} />
        <div class="conversation-title">
          <h1>{state().title}</h1>
          <p title={state().cwd}>
            {doing()} · {folder(state().cwd)}
            {state().model ? ` · ${state().model}` : ''}
          </p>
        </div>
        <button
          type="button"
          class="button"
          aria-pressed={props.filesOpen}
          title="What Pi wrote and changed in this conversation"
          onClick={props.onFiles}
        >
          <Icon name="file" size={13} />
          Files
          <span class="count">{state().files.length}</span>
        </button>
        <Show when={!state().stopped}>
          <button
            type="button"
            class="button button-text"
            title="Stop Pi for this scratchpad. It stays in the list, and opens again where it left off."
            disabled={close.busy()}
            onClick={() => void close.run()}
          >
            Close
          </button>
        </Show>
      </header>
      <ErrorNote error={close.error()} />

      <div
        class="log"
        ref={log}
        onScroll={() => {
          following = log.scrollHeight - log.scrollTop - log.clientHeight < 48;
        }}
      >
        <div class="log-content" ref={content}>
          <For each={props.session.entries}>{(entry) => <EntryView entry={entry} />}</For>
          <Show when={props.session.entries.length === 0 && !state().busy}>
            <p class="log-empty">Ask Pi anything. It works in {state().cwd}.</p>
          </Show>
        </div>
      </div>

      <div class="conversation-foot">
        <Show when={state().stopped}>
          <div class="stopped" role="alert">
            <span>{state().stopped}</span>
            <Show when={state().file}>
              <button
                type="button"
                class="button"
                disabled={restart.busy()}
                onClick={() => void restart.run()}
              >
                Start Pi again
              </button>
            </Show>
          </div>
          <ErrorNote error={restart.error()} />
        </Show>
        <Show when={state().ask} keyed>
          {(ask) => <AskCard session={props.session.id} ask={ask} />}
        </Show>
        <Composer session={props.session} />
        <div class="status-line mono">
          <span title={state().cwd}>{state().cwd}</span>
          <For each={state().statuses}>{(line) => <span>{line.text}</span>}</For>
        </div>
      </div>
    </main>
  );
}
