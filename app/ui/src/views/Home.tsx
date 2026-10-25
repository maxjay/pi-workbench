// The first tab: ask Pi something, and see scratchpads, codespaces, tasks
// and pull requests at a glance. Each list fills in as its stage lands.

import { createSignal, For, Show } from 'solid-js';
import { status, stopReason } from '../agent';
import { createAction } from '../components/action';
import { Icon } from '../components/Icon';
import { Empty, ErrorNote } from '../components/Note';
import type { TaskView } from '../protocol/TaskView';
import { health } from '../state/health';
import { scratchpads, showScratchpad, startScratchpad } from '../state/sessions';
import { open } from '../state/tabs';
import { allPaused, pauseAll, standing as taskStanding, tasks } from '../state/tasks';
import { standing } from './scratch/Nav';
import { scheduleWords, timeWords } from './task/words';

export function Home() {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    await startScratchpad(text().trim());
    setText('');
    open('scratch');
  });
  const show = createAction(async (pad: Parameters<typeof showScratchpad>[0]) => {
    await showScratchpad(pad);
    open('scratch');
  });
  return (
    <div class="page home">
      <section class="ask">
        <h1>What are we working on?</h1>
        <textarea
          class="ask-box"
          rows={3}
          placeholder="Ask Pi anything. It starts a scratchpad."
          value={text()}
          disabled={start.busy() || status() !== 'ready'}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && text().trim()) {
              e.preventDefault();
              void start.run();
            }
          }}
        />
        <ErrorNote error={start.error()} />
      </section>

      <section class="list">
        <h2>Scratchpads</h2>
        <ErrorNote error={show.error()} />
        <Show when={scratchpads().length > 0} fallback={<Empty>Nothing yet. Ask Pi something above.</Empty>}>
          <ul class="rows">
            <For each={scratchpads().slice(0, 5)}>
              {(pad) => (
                <li>
                  <button type="button" class="row row-button" onClick={() => void show.run(pad)}>
                    <span class="pad-dot" data-state={standing(pad)} />
                    <span class="row-title">{pad.title}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      <section class="list">
        <h2>Codespaces</h2>
        <Empty>No codespaces yet. Opening a folder in WSL as a codespace comes in stage 3.</Empty>
      </section>

      <Tasks />

      <section class="list">
        <h2>Pull requests</h2>
        <Empty>Nothing to review. Pull requests will show here once dova is plugged in.</Empty>
      </section>

      <AgentCard />
    </div>
  );
}

/** What a task's row says on the right, and how loudly. */
function last(view: TaskView) {
  const now = taskStanding(view);
  if (now === 'asking') return { text: 'Needs you', tone: 'asking' };
  if (now === 'running') return { text: 'Running…', tone: 'running' };
  if (view.task.paused) return { text: 'Paused', tone: 'paused' };
  const run = view.runs[0];
  if (!run) return { text: 'Not run yet', tone: '' };
  if (now === 'failed') return { text: `Failed ${timeWords(run.started)}`, tone: 'failed' };
  if (now === 'new') return { text: 'New run', tone: 'new' };
  return { text: `${run.state === 'failed' ? 'Failed' : 'Done'} ${timeWords(run.started)}`, tone: '' };
}

function Tasks() {
  const resume = createAction(() => pauseAll(false));
  return (
    <section class="list" aria-labelledby="tasks">
      <div class="list-head">
        <h2 id="tasks">Tasks</h2>
        <span class="count">Scheduled prompts</span>
        <span class="list-head-gap" />
        <button type="button" class="button button-text button-small" onClick={() => open('newtask')}>
          New task
        </button>
      </div>
      <Show when={allPaused()}>
        <div class="paused-all" role="status">
          <span>All tasks paused. Runs they miss meanwhile happen when you resume.</span>
          <button
            type="button"
            class="button button-small"
            disabled={resume.busy()}
            onClick={() => void resume.run()}
          >
            Resume
          </button>
        </div>
        <ErrorNote error={resume.error()} />
      </Show>
      <Show
        when={tasks().length > 0}
        fallback={<Empty>No tasks yet. Make one to have Pi run a prompt on a schedule.</Empty>}
      >
        <ul class="rows">
          <For each={tasks()}>
            {(view) => (
              <li>
                <button
                  type="button"
                  class="row row-button task-row"
                  data-tone={last(view).tone}
                  onClick={() => open(`task:${view.task.id}`)}
                >
                  <Icon name="clock" size={15} />
                  <span class="row-main">
                    <span class="row-title">{view.task.name}</span>
                    <span class="row-detail">{scheduleWords(view.task.schedule)}</span>
                  </span>
                  <span class="task-last">{last(view).text}</span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}

function AgentCard() {
  return (
    <section class="list">
      <h2>wb-agent</h2>
      <Show
        when={status() !== 'down'}
        fallback={
          <p class="note note-error">{stopReason() ?? 'wb-agent stopped'}. Restart it from Ctrl K.</p>
        }
      >
        <ErrorNote error={health.error} />
        <Show when={!health.error && health()} fallback={<Empty>Starting…</Empty>}>
          {(h) => (
            <dl class="facts">
              <dt>Version</dt>
              <dd>{h().version}</dd>
              <dt>Runs in</dt>
              <dd>{h().distro ?? 'Linux, outside WSL'}</dd>
              <dt>Pi</dt>
              <dd>{h().pi ?? 'not found on PATH'}</dd>
              <dt>Pi’s files</dt>
              <dd class="mono">{h().agentDir}</dd>
            </dl>
          )}
        </Show>
      </Show>
    </section>
  );
}
