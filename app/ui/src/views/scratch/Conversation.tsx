// A scratchpad's conversation is the page, as with Pi in a terminal: what
// you asked, what Pi thought, ran and said. It follows the newest line
// unless you've scrolled up to read. While Pi asks something, its card
// takes the place of the box you write in.

import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { draftKey, setDraft } from '../../state/drafts';
import { reopen, setDone, unqueue } from '../../state/sessions';
import { AskCard } from './AskCard';
import { Composer } from './Composer';
import { EntryView } from './EntryView';
import { todos } from './tools';
import { usage } from './usage';

/** Where the session stands, in a word or two, and its dot. */
function standing(session: Session) {
  const state = session.state;
  if (state.ask) return { word: 'Waiting for you', dot: 'asking' };
  if (state.busy) return { word: 'Working', dot: 'busy' };
  if (state.stopped) return { word: 'Pi stopped', dot: 'stopped' };
  if (state.done) return { word: 'Done', dot: 'done' };
  return { word: 'Idle', dot: 'idle' };
}

export function Conversation(props: {
  session: Session;
  filesOpen: boolean;
  onFiles: () => void;
  onOpenFile: (path: string) => void;
}) {
  const state = () => props.session.state;
  const [planOpen, setPlanOpen] = createSignal(false);
  const plan = () => todos(props.session.entries);
  const done = createAction(async () => {
    const file = state().file;
    if (file) await setDone(file, true);
  });
  const restart = createAction(async () => {
    const file = state().file;
    if (file) await reopen(file);
  });
  const takeBack = createAction(async (index: number, edit: boolean) => {
    const item = state().queued[index];
    await unqueue(props.session.id, index);
    if (edit && item) setDraft(draftKey(props.session), item.text);
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

  return (
    <main class="conversation">
      <header class="conversation-head">
        <div class="conversation-bar">
          <span class="pad-dot pad-dot-large" data-state={standing(props.session).dot} />
          <div class="conversation-title">
            <h1>{state().title}</h1>
            <p title={state().cwd}>{standing(props.session).word} · no codespace</p>
          </div>
          <Show when={plan()}>
            {(steps) => (
              <button
                type="button"
                class="button"
                aria-expanded={planOpen()}
                onClick={() => setPlanOpen(!planOpen())}
              >
                Plan
                <span class="count">
                  {steps().filter((step) => step.done).length}/{steps().length}
                </span>
                <span class="chevron" data-open={planOpen()}>
                  <Icon name="chevron" size={10} />
                </span>
              </button>
            )}
          </Show>
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
          {/* With the viewer open, the header keeps to Plan and Files. */}
          <Show when={!props.filesOpen && !state().done && state().file}>
            <button
              type="button"
              class="button button-text"
              title="Pi stops; writing here again starts it"
              disabled={done.busy()}
              onClick={() => void done.run()}
            >
              Mark done
            </button>
          </Show>
        </div>
        <Show when={planOpen() && plan()}>
          {(steps) => (
            <ol class="plan">
              <For each={steps()}>
                {(step, i) => (
                  <li
                    data-done={step.done}
                    data-working={!step.done && state().busy && steps().findIndex((s) => !s.done) === i()}
                  >
                    <span class="plan-ring">
                      <Show when={step.done}>
                        <Icon name="check" size={10} />
                      </Show>
                    </span>
                    {step.text}
                  </li>
                )}
              </For>
            </ol>
          )}
        </Show>
      </header>
      <ErrorNote error={done.error()} />

      <div
        class="log"
        ref={log}
        onScroll={() => {
          following = log.scrollHeight - log.scrollTop - log.clientHeight < 48;
        }}
      >
        <div class="log-content" ref={content}>
          <For each={props.session.entries}>
            {(entry, i) => (
              <EntryView
                entry={entry}
                cwd={state().cwd}
                last={i() === props.session.entries.length - 1 && state().busy}
                onOpenFile={props.onOpenFile}
              />
            )}
          </For>
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
    </main>
  );
}
