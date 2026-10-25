// A task's runs down the left, newest first and grouped by day: each with
// a dot for how it went, when it started, and a line on what it did.

import { For, Show } from 'solid-js';
import type { Run } from '../../protocol/Run';
import type { TaskView } from '../../protocol/TaskView';
import { allPaused, going } from '../../state/tasks';
import { dayWords, timeWords, whenWords } from './words';

/** What a run's row says. */
export function runLine(run: Run) {
  if (run.state === 'asking') return 'Needs you';
  if (run.state === 'running') return 'Running…';
  if (run.state === 'stopped') return 'Stopped by you';
  return run.summary || (run.state === 'failed' ? 'Failed' : 'Done');
}

export function RunList(props: { view: TaskView; picked?: string; onPick: (id: string) => void }) {
  const next = () => {
    const now = going(props.view);
    if (now) return now.state === 'asking' ? 'Waiting for you' : 'Running now';
    if (props.view.task.paused || allPaused()) return 'Paused';
    return props.view.next ? `Next ${whenWords(props.view.next)}` : 'Runs when you ask';
  };
  const firstRun = () =>
    props.view.next ? `The first is ${whenWords(props.view.next)}.` : 'Run it when you’re ready.';

  return (
    <nav class="runs" aria-label="Runs">
      <div class="runs-head">
        <span>Runs</span>
        <span class="count">{next()}</span>
      </div>
      <div class="runs-list">
        <Show when={props.view.runs.length === 0}>
          <p class="runs-empty">No runs yet. {firstRun()}</p>
        </Show>
        <For each={props.view.runs}>
          {(run, i) => (
            <>
              <Show when={i() === 0 || dayWords(props.view.runs[i() - 1].started) !== dayWords(run.started)}>
                <p class="runs-day">{dayWords(run.started)}</p>
              </Show>
              <button
                type="button"
                class="run-row"
                aria-current={run.id === props.picked ? 'true' : undefined}
                onClick={() => props.onPick(run.id)}
              >
                <span class="run-dot" data-state={run.state} aria-hidden="true" />
                <span class="run-time mono">{timeWords(run.started)}</span>
                <span class="run-summary" data-state={run.state}>
                  {runLine(run)}
                </span>
              </button>
            </>
          )}
        </For>
      </div>
    </nav>
  );
}
