// A scratchpad's conversation is the page, as with Pi in a terminal: its
// title, plan and actions on top, then the parts every conversation has:
// the log, and the foot with the box you write in.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { setDone } from '../../state/sessions';
import { Foot } from '../chat/Foot';
import { Log } from '../chat/Log';
import { todos } from '../chat/tools';

/** Where the session stands, in a word or two, and its dot. */
export function standing(session: Session) {
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
      <Log session={props.session} onOpenFile={props.onOpenFile} />
      <Foot session={props.session} />
    </main>
  );
}
