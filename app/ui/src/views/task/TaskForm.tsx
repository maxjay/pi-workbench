// Making or changing a task: its name, what Pi does each run, when it
// runs, and whether runs share a conversation. Pi can fill in the schedule
// from words; the controls below are what actually runs. Nothing changes
// until it's saved.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { Schedule } from '../../protocol/Schedule';
import type { TaskView } from '../../protocol/TaskView';
import { close, replace } from '../../state/tabs';
import { saveTask, scheduleFrom } from '../../state/tasks';
import { days, scheduleWords } from './words';

type Kind = Schedule['kind'];

export function TaskForm(props: { view?: TaskView; onDone?: () => void }) {
  const task = props.view?.task;
  const old = task?.schedule;
  const [name, setName] = createSignal(task?.name ?? '');
  const [prompt, setPrompt] = createSignal(task?.prompt ?? '');
  const [kind, setKind] = createSignal<Kind>(old?.kind ?? 'at');
  const [picked, setPicked] = createSignal<number[]>(old?.kind === 'at' ? old.days : [0, 1, 2, 3, 4]);
  const [time, setTime] = createSignal(old?.kind === 'at' ? old.time : '09:00');
  const [hours, setHours] = createSignal(old?.kind === 'every' ? old.hours : 2);
  const [continuing, setContinuing] = createSignal(task?.continuing ?? true);
  const [words, setWords] = createSignal('');
  const [filled, setFilled] = createSignal<string>();

  const schedule = (): Schedule =>
    kind() === 'at'
      ? { kind: 'at', days: picked(), time: time() }
      : kind() === 'every'
        ? { kind: 'every', hours: hours() }
        : { kind: 'manual' };

  const step = (by: number) => {
    const [h, m] = time().split(':').map(Number);
    const total = (h * 60 + m + by + 1440) % 1440;
    setTime(`${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`);
  };

  const fill = createAction(async () => {
    if (!words().trim()) return;
    const found = await scheduleFrom(words().trim());
    setKind(found.kind);
    if (found.kind === 'at') {
      setPicked(found.days);
      setTime(found.time);
    } else if (found.kind === 'every') setHours(found.hours);
    setFilled(`Pi set it to: ${scheduleWords(found)}. Check it below; the controls are what runs.`);
  });

  const leave = () => (task ? props.onDone?.() : close('newtask'));
  const save = createAction(async () => {
    const id = await saveTask({
      id: task?.id ?? null,
      name: name(),
      prompt: prompt(),
      schedule: schedule(),
      continuing: continuing(),
    });
    if (task) props.onDone?.();
    else if (id) replace('newtask', `task:${id}`);
  });

  return (
    <section class="task-form" aria-label={task ? 'Edit task' : 'New task'}>
      <div class="task-form-head">
        <button
          type="button"
          class="icon-button"
          aria-label={task ? 'Back to the task' : 'Close without making it'}
          title={task ? 'Back to the task' : 'Close without making it'}
          onClick={leave}
        >
          <Icon name="back" size={16} />
        </button>
        <div class="task-form-title">
          <h1>{task ? 'Edit task' : 'New task'}</h1>
          <p>{task ? task.name : 'A prompt Pi runs on a schedule, in its own conversation'}</p>
        </div>
        <button type="button" class="button" onClick={leave}>
          Cancel
        </button>
        <button
          type="button"
          class="button button-primary"
          disabled={save.busy()}
          onClick={() => void save.run()}
        >
          {task ? 'Save changes' : 'Make task'}
        </button>
      </div>
      <form
        class="task-form-body"
        onSubmit={(e) => {
          e.preventDefault();
          void save.run();
        }}
      >
        <ErrorNote error={save.error()} />
        <label class="form-field">
          <span>Name</span>
          <input class="input input-large" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
        </label>
        <label class="form-field">
          <span>What Pi does each run</span>
          <textarea
            class="input"
            rows={5}
            value={prompt()}
            onInput={(e) => setPrompt(e.currentTarget.value)}
          />
        </label>

        <fieldset class="form-field">
          <legend>When it runs</legend>
          <div class="field-row">
            <input
              class="input"
              aria-label="Describe when it should run"
              placeholder="Say it in words, e.g. weekdays first thing in the morning"
              value={words()}
              onInput={(e) => setWords(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  void fill.run();
                }
              }}
            />
            <button
              type="button"
              class="button"
              disabled={fill.busy() || !words().trim()}
              onClick={() => void fill.run()}
            >
              {fill.busy() ? 'Asking Pi…' : 'Fill in with Pi'}
            </button>
          </div>
          <Show when={filled()}>
            <span class="field-note">{filled()}</span>
          </Show>
          <ErrorNote error={fill.error()} />
          <fieldset class="segmented segmented-large">
            <legend class="visually-hidden">Kind of schedule</legend>
            <For
              each={
                [
                  ['at', 'At set times'],
                  ['every', 'Every few hours'],
                  ['manual', 'Only when I run it'],
                ] as [Kind, string][]
              }
            >
              {([value, label]) => (
                <button type="button" aria-pressed={kind() === value} onClick={() => setKind(value)}>
                  {label}
                </button>
              )}
            </For>
          </fieldset>
          <Show when={kind() === 'at'}>
            <div class="field-row">
              <For each={days}>
                {(label, i) => (
                  <button
                    type="button"
                    class="day"
                    aria-pressed={picked().includes(i())}
                    onClick={() =>
                      setPicked(
                        picked().includes(i()) ? picked().filter((d) => d !== i()) : [...picked(), i()],
                      )
                    }
                  >
                    {label}
                  </button>
                )}
              </For>
              <span class="field-gap">at</span>
              <fieldset class="stepper">
                <legend class="visually-hidden">Time</legend>
                <button type="button" aria-label="Half an hour earlier" onClick={() => step(-30)}>
                  −
                </button>
                <span class="mono">{time()}</span>
                <button type="button" aria-label="Half an hour later" onClick={() => step(30)}>
                  +
                </button>
              </fieldset>
            </div>
          </Show>
          <Show when={kind() === 'every'}>
            <div class="field-row">
              <span class="field-gap">Every</span>
              <fieldset class="stepper">
                <legend class="visually-hidden">How often</legend>
                <button
                  type="button"
                  aria-label="Less often"
                  onClick={() => setHours(Math.max(1, hours() - 1))}
                >
                  −
                </button>
                <span class="mono">{hours()}</span>
                <button
                  type="button"
                  aria-label="More often"
                  onClick={() => setHours(Math.min(24, hours() + 1))}
                >
                  +
                </button>
              </fieldset>
              <span class="field-gap">{hours() === 1 ? 'hour' : 'hours'}</span>
            </div>
          </Show>
          <span class="field-summary">{scheduleWords(schedule())}</span>
        </fieldset>

        <fieldset class="form-field">
          <legend>Conversation</legend>
          <For
            each={
              [
                [
                  true,
                  'Continue one conversation',
                  'Every run adds to the same conversation, so you can ask about earlier days.',
                ],
                [
                  false,
                  'Fresh each run',
                  'Each run starts clean; ask about a run in that run’s own conversation.',
                ],
              ] as [boolean, string, string][]
            }
          >
            {([value, label, detail]) => (
              <button
                type="button"
                class="choice"
                aria-pressed={continuing() === value}
                onClick={() => setContinuing(value)}
              >
                <span class="choice-ring" aria-hidden="true" />
                <span class="choice-text">
                  <span>{label}</span>
                  <span class="count">{detail}</span>
                </span>
              </button>
            )}
          </For>
        </fieldset>

        <div class="field-row field-row-top">
          <label class="form-field field-half">
            <span>Runs in</span>
            <select class="input" disabled>
              <option>No codespace</option>
            </select>
          </label>
          <div class="form-field field-half">
            <span>Writes to</span>
            <span class="input input-static mono" title={task?.folder}>
              {task ? task.folder : 'A folder of its own'}
            </span>
          </div>
        </div>
      </form>
    </section>
  );
}
