// A task's conversation, beside its runs: one across every run when the
// task continues one, or the picked run's own. Each run starts with a
// header saying when and how it went. Ask Pi about a run here; when a run
// waits on you, its question takes the place of the box you write in.

import { For, Show } from 'solid-js';
import type { Run } from '../../protocol/Run';
import type { Session } from '../../protocol/Session';
import type { TaskView } from '../../protocol/TaskView';
import { AskCard } from '../chat/AskCard';
import { Composer } from '../chat/Composer';
import { EntryView } from '../chat/EntryView';
import { dayWords, timeWords, whenWords } from './words';

const states: Record<Run['state'], string> = {
  running: 'Running',
  asking: 'Waiting for you',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
};
const triggers: Record<Run['trigger'], string> = { scheduled: 'Scheduled', late: 'Late', now: 'Run now' };

export function TaskChat(props: {
  view: TaskView;
  session?: Session;
  picked?: string;
  onPick: (id: string) => void;
}) {
  /** The runs in this conversation, by where each starts. */
  const starts = () =>
    new Map(
      props.view.runs
        .filter((run) => props.session && run.session === props.session.state.file)
        .map((run) => [run.from, run]),
    );
  const meta = () => {
    if (!props.session) return '';
    if (props.view.task.continuing) {
      const n = props.view.runs.length;
      return `One conversation · ${n} ${n === 1 ? 'run' : 'runs'}`;
    }
    const run = props.view.runs.find((r) => r.id === props.picked);
    return run ? `${dayWords(run.started)} ${timeWords(run.started)} run’s conversation` : '';
  };

  return (
    <aside class="task-chat" aria-label="Conversation with Pi">
      <div class="task-chat-head">
        <span class="task-chat-title">{props.view.task.name}</span>
        <span class="count">{meta()}</span>
      </div>
      {/* Newest at the bottom, where it opens. */}
      <div class="task-chat-log">
        <div class="task-chat-content">
          <Show
            when={props.session}
            fallback={
              <p class="log-empty">
                {props.view.task.continuing
                  ? 'Each run goes in this conversation, so you can ask about any of them here.'
                  : 'Each run has a conversation of its own. Pick one to ask about it here.'}{' '}
                {props.view.next ? `The first is ${whenWords(props.view.next)}.` : ''}
              </p>
            }
          >
            {(session) => (
              <For each={session().entries}>
                {(entry, i) => (
                  <Show
                    when={starts().get(i())}
                    fallback={
                      <EntryView
                        entry={entry}
                        cwd={session().state.cwd}
                        last={i() === session().entries.length - 1 && session().state.busy}
                        onOpenFile={() => {}}
                      />
                    }
                  >
                    {(run) => (
                      <div class="chat-run" data-state={run().state} data-picked={run().id === props.picked}>
                        <div class="chat-run-head">
                          <strong>Run · {`${dayWords(run().started)} ${timeWords(run().started)}`}</strong>
                          <span class="count">{triggers[run().trigger]}</span>
                          <span class="chat-run-state">{states[run().state]}</span>
                        </div>
                        <Show when={run().id !== props.picked}>
                          <button
                            type="button"
                            class="button button-text button-small"
                            onClick={() => props.onPick(run().id)}
                          >
                            Show this run
                          </button>
                        </Show>
                      </div>
                    )}
                  </Show>
                )}
              </For>
            )}
          </Show>
        </div>
      </div>
      <Show when={props.session}>
        {(session) => (
          <div class="task-chat-foot">
            <Show when={session().state.ask} keyed fallback={<Composer session={session()} />}>
              {(ask) => <AskCard session={session().id} ask={ask} />}
            </Show>
          </div>
        )}
      </Show>
    </aside>
  );
}
