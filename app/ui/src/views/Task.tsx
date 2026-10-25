// A task's tab: its runs down the left; the task and the run you picked in
// the middle, or the form while you change it; its conversation on the
// right. A new task's tab is the form alone.

import { createEffect, createResource, createSignal, Show } from 'solid-js';
import { createStore } from 'solid-js/store';
import { createAction } from '../components/action';
import { Icon } from '../components/Icon';
import { ErrorNote } from '../components/Note';
import type { Run } from '../protocol/Run';
import type { Session } from '../protocol/Session';
import type { TaskView } from '../protocol/TaskView';
import { follow, live, readTaskSession } from '../state/sessions';
import { allPaused, going, markSeen, runTask, setTaskPaused, stopTask, taskById } from '../state/tasks';
import { RunList } from './task/RunList';
import { RunOutput } from './task/RunOutput';
import { TaskChat } from './task/TaskChat';
import { TaskForm } from './task/TaskForm';
import { scheduleWords, whenWords } from './task/words';

/** The run picked in each task, while its tab is open. */
const [picked, setPicked] = createStore<Record<string, string | undefined>>({});
/** Tasks whose form is open in their tab. */
const [editing, setEditing] = createSignal<ReadonlySet<string>>(new Set());

export function NewTask() {
  return (
    <div class="page task-page">
      <TaskForm />
    </div>
  );
}

export function TaskPage(props: { id: string }) {
  return (
    <Show when={taskById(props.id)} fallback={<div class="page task-page" />}>
      {(view) => <Page view={view()} />}
    </Show>
  );
}

/**
 * The session a run is in: live while Pi runs it, else read from its file.
 * Undefined until it has one.
 */
function useSession(run: () => Run | undefined) {
  const [saved] = createResource(
    () => {
      const r = run();
      if (!r || r.live || !r.session) return undefined;
      // Read again once a run has finished, as it adds to the file.
      return { file: r.session, finished: r.finished };
    },
    ({ file }) => readTaskSession(file),
  );
  createEffect(() => {
    const id = run()?.live;
    if (id && !live[id]) void follow(id).catch(() => {});
  });
  return (): Session | undefined => {
    const r = run();
    if (!r) return undefined;
    if (r.live && live[r.live]) return live[r.live];
    const following = r.session ? Object.values(live).find((s) => s.state.file === r.session) : undefined;
    return following ?? saved.latest;
  };
}

function Page(props: { view: TaskView }) {
  const task = () => props.view.task;
  const runs = () => props.view.runs;
  const run = () => runs().find((r) => r.id === picked[task().id]) ?? runs()[0];
  const session = useSession(run);
  // A continuing task's conversation is every run's; otherwise it's the picked run's.
  const chatRun = () => (task().continuing ? (runs().find((r) => r.session) ?? run()) : run());
  const chatSession = useSession(chatRun);
  const isEditing = () => editing().has(task().id);

  // Looking at a task's runs means its newest isn't new any more.
  createEffect(() => {
    if (runs()[0]?.finished) markSeen(task().id);
  });

  return (
    <div class="page task-page">
      <RunList
        view={props.view}
        picked={run()?.id}
        onPick={(id) => {
          setPicked(task().id, id);
          setEditing(new Set([...editing()].filter((t) => t !== task().id)));
        }}
      />
      <div class="task-middle">
        <Show
          when={!isEditing()}
          fallback={
            <TaskForm
              view={props.view}
              onDone={() => setEditing(new Set([...editing()].filter((t) => t !== task().id)))}
            />
          }
        >
          <Header view={props.view} onEdit={() => setEditing(new Set([...editing(), task().id]))} />
          <RunOutput view={props.view} run={run()} session={session()} />
        </Show>
      </div>
      <TaskChat
        view={props.view}
        session={chatSession()}
        picked={run()?.id}
        onPick={(id) => setPicked(task().id, id)}
      />
    </div>
  );
}

function Header(props: { view: TaskView; onEdit: () => void }) {
  const task = () => props.view.task;
  const now = () => going(props.view);
  const last = () => props.view.runs.find((r) => r.finished !== null);
  const act = createAction(async (work: () => Promise<unknown>) => work());
  const status = () => {
    const run = now();
    if (run?.state === 'asking') return { text: 'Waiting for your answer', tone: 'asking' };
    if (run) return { text: 'Running now', tone: 'running' };
    const next = props.view.next;
    if (task().paused || allPaused())
      return { text: next ? `Next would be ${whenWords(next)}` : 'Won’t run until you resume it', tone: '' };
    const done = last();
    if (!done) return { text: 'Hasn’t run yet', tone: '' };
    if (done.state === 'failed') return { text: `Last run failed: ${done.summary}`, tone: 'failed' };
    return { text: `Last ran ${whenWords(done.started)} · ${done.summary}`, tone: '' };
  };
  return (
    <section class="task-head" aria-label="Task">
      <div class="task-title">
        <h1>{task().name}</h1>
        <button
          type="button"
          class="button button-text"
          title="Change the prompt and the schedule"
          onClick={props.onEdit}
        >
          <Icon name="pencil" size={13} />
          Edit task
        </button>
      </div>
      <p class="task-meta">
        <span class="task-when">{scheduleWords(task().schedule)}</span>
        <span aria-hidden="true">·</span>
        <span>{task().continuing ? 'One conversation across runs' : 'Fresh each run'}</span>
        <span aria-hidden="true">·</span>
        <span>No codespace</span>
      </p>
      <div class="task-bar">
        <Show when={task().paused || allPaused()}>
          <span class="pill">{allPaused() && !task().paused ? 'All paused' : 'Paused'}</span>
        </Show>
        <span class="task-status" data-tone={status().tone}>
          {status().text}
        </span>
        <Show
          when={now()}
          fallback={
            <button
              type="button"
              class="button"
              disabled={act.busy()}
              onClick={() => void act.run(() => setTaskPaused(task().id, !task().paused))}
            >
              {task().paused ? 'Resume' : 'Pause'}
            </button>
          }
        >
          <button
            type="button"
            class="button"
            disabled={act.busy()}
            onClick={() => void act.run(() => stopTask(task().id))}
          >
            Stop
          </button>
        </Show>
        <button
          type="button"
          class="button button-primary"
          disabled={act.busy() || now() !== undefined}
          onClick={() => void act.run(() => runTask(task().id))}
        >
          <Icon name="play" size={11} />
          {now() ? 'Running…' : 'Run now'}
        </button>
      </div>
      <ErrorNote error={act.error()} />
    </section>
  );
}
