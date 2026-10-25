// Tasks and their runs, as wb-agent reports them: it keeps them, runs them
// on their schedules and says whenever anything changes. This keeps the
// latest it said, which runs you've looked at, and asks it to act.

import { createResource, createRoot, createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { ask, onEvent, status } from '../agent';
import type { Run } from '../protocol/Run';
import type { TaskInput } from '../protocol/TaskInput';
import type { TaskList } from '../protocol/TaskList';
import type { TaskView } from '../protocol/TaskView';

const [pushed, setPushed] = createSignal<TaskList>();
const [loaded] = createRoot(() =>
  createResource(
    () => status() === 'ready',
    async () => {
      const list = (await ask({ method: 'tasks.list' }, 'tasks')).list;
      setPushed(list);
      return list;
    },
  ),
);

onEvent((event) => {
  if (event.event === 'tasks') setPushed(event.list);
});

/** Every task, as wb-agent last said. */
export const tasks = (): TaskView[] => (pushed() ?? loaded.latest)?.tasks ?? [];
export const allPaused = () => (pushed() ?? loaded.latest)?.paused ?? false;
export const tasksLoading = () => loaded.loading;
export const taskById = (id: string) => tasks().find((view) => view.task.id === id);

const keep = (list: TaskList) => {
  setPushed(list);
  return list;
};

/** Makes or changes a task, and gives its id. */
export async function saveTask(input: TaskInput) {
  const before = new Set(tasks().map((view) => view.task.id));
  const list = keep((await ask({ method: 'tasks.save', params: { task: input } }, 'tasks')).list);
  return input.id ?? list.tasks.find((view) => !before.has(view.task.id))?.task.id;
}

export async function setTaskPaused(id: string, paused: boolean) {
  keep((await ask({ method: 'tasks.setPaused', params: { id, paused } }, 'tasks')).list);
}

export async function pauseAll(paused: boolean) {
  keep((await ask({ method: 'tasks.pauseAll', params: { paused } }, 'tasks')).list);
}

export async function runTask(id: string) {
  keep((await ask({ method: 'tasks.run', params: { id } }, 'tasks')).list);
}

export async function stopTask(id: string) {
  keep((await ask({ method: 'tasks.stop', params: { id } }, 'tasks')).list);
}

/** A schedule from a description in words, worked out by Pi. */
export async function scheduleFrom(words: string) {
  return (await ask({ method: 'tasks.scheduleFrom', params: { words } }, 'schedule')).schedule;
}

// The last run you looked at in each task, so a new one stands out. Only
// the window needs it, so it's kept in the window's own storage.

const SEEN = 'pi-workbench.seen-runs';

function loadSeen(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(SEEN) ?? '{}') ?? {};
  } catch {
    return {};
  }
}

const [seen, setSeen] = createStore<Record<string, string>>(loadSeen());

/** You've looked at a task's runs: the newest isn't new any more. */
export function markSeen(id: string) {
  const newest = taskById(id)?.runs.find((run) => run.finished !== null);
  if (!newest || seen[id] === newest.id) return;
  setSeen(id, newest.id);
  try {
    localStorage.setItem(SEEN, JSON.stringify(seen));
  } catch {
    // Storage can be full or off; it only means a run shows as new again.
  }
}

/** The run that's going, if one is. */
export const going = (view: TaskView): Run | undefined =>
  view.runs.find((run) => run.state === 'running' || run.state === 'asking');

/**
 * Where a task stands, as Home and its tab highlight it: waiting on you,
 * running, failed or new since you last looked, or nothing to say.
 */
export function standing(view: TaskView): 'asking' | 'running' | 'failed' | 'new' | undefined {
  const now = going(view);
  if (now) return now.state === 'asking' ? 'asking' : 'running';
  const last = view.runs[0];
  if (!last || seen[view.task.id] === last.id) return undefined;
  return last.state === 'failed' ? 'failed' : last.state === 'done' ? 'new' : undefined;
}
