// The run you picked: the note it wrote, its steps while it goes, its
// question while it waits on you, or why it failed.

import { createResource, For, Match, Show, Switch } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import type { Entry } from '../../protocol/Entry';
import type { Run } from '../../protocol/Run';
import type { Session } from '../../protocol/Session';
import type { TaskView } from '../../protocol/TaskView';
import { readFile } from '../../state/sessions';
import { summarize } from '../chat/tools';
import { dayWords, timeWords, whenWords } from './words';

/** The run's part of its conversation. */
export function runEntries(session: Session | undefined, run: Run, runs: Run[]) {
  if (!session) return [];
  const later = runs
    .filter((r) => r.session === run.session && r.from > run.from)
    .reduce((end, r) => Math.min(end, r.from), session.entries.length);
  return session.entries.slice(run.from, later);
}

/** What Pi did in a run, a line a step: the calls it made. */
export function steps(entries: Entry[]) {
  return entries
    .filter((entry) => entry.kind === 'tool')
    .map((entry) => {
      const line = summarize(entry);
      return { text: `${line.verb} ${line.target}`.trim(), status: entry.status };
    });
}

const name = (path: string) => path.split('/').pop() ?? path;
const folderOf = (path: string, root: string) => {
  const inside = path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path;
  const cut = inside.lastIndexOf('/');
  return cut > 0 ? `${inside.slice(0, cut)}/` : '';
};

export function RunOutput(props: { view: TaskView; run?: Run; session?: Session }) {
  const entries = () => (props.run ? runEntries(props.session, props.run, props.view.runs) : []);
  const note = () => props.run?.files.at(-1);
  const [file] = createResource(
    () => (props.run?.state === 'done' && note() ? { path: note() as string, run: props.run.id } : undefined),
    ({ path }) => readFile(props.session ?? ({ id: '' } as Session), path),
  );
  const reply = () =>
    [...entries()].reverse().find((entry): entry is Extract<Entry, { kind: 'text' }> => entry.kind === 'text')
      ?.text;
  const question = () => {
    const ask = props.session?.state.ask;
    if (!ask) return '';
    return ask.kind.type === 'questions' ? ask.kind.questions[0]?.question : ask.title;
  };
  const when = () => (props.run ? `${dayWords(props.run.started)} · ${timeWords(props.run.started)}` : '');

  return (
    <section class="run-output" aria-label="Run output">
      <div class="run-output-head">
        <Show
          when={props.run?.state === 'done' && note()}
          fallback={<span class="run-output-name">{when()}</span>}
        >
          {(path) => (
            <>
              <span class="count">{folderOf(path(), props.view.task.folder)}</span>
              <span class="run-output-name">{name(path())}</span>
              <span class="run-output-when">{when()}</span>
            </>
          )}
        </Show>
      </div>
      <div class="run-output-body">
        <Switch>
          <Match when={!props.run}>
            <div class="run-empty">
              <strong>Nothing yet</strong>
              <span>
                It hasn’t run yet. Run it now to see what Pi writes
                {props.view.next ? `, or wait for ${whenWords(props.view.next)}` : ''}.
              </span>
            </div>
          </Match>
          <Match when={props.run?.state === 'running'}>
            <div class="run-going">
              <span class="run-going-note">Pi is running the task. Its note lands here when it’s done.</span>
              <Steps entries={entries()} />
            </div>
          </Match>
          <Match when={props.run?.state === 'asking'}>
            <div class="run-going">
              <span class="run-going-note" data-tone="asking">
                Pi is waiting for your answer. The run carries on once you’ve given it.
              </span>
              <strong class="run-question">{question()}</strong>
              <Steps entries={entries()} asking />
              <span class="count">Answer it in the conversation on the right.</span>
            </div>
          </Match>
          <Match when={props.run?.state === 'failed' || props.run?.state === 'stopped'}>
            <div class="run-failed" data-state={props.run?.state} role="status">
              <strong>{props.run?.state === 'stopped' ? 'You stopped this run' : 'This run failed'}</strong>
              <Show when={props.run?.error}>
                <pre class="mono">{props.run?.error}</pre>
              </Show>
              <span>
                {props.run?.state === 'stopped'
                  ? 'Run it again when you’re ready.'
                  : 'The conversation on the right has what Pi did up to then.'}
              </span>
            </div>
          </Match>
          <Match when={props.run?.state === 'done'}>
            <article class="run-note">
              <Show
                when={note() && file.latest?.content.type === 'text' ? file.latest.content : undefined}
                fallback={
                  <Show when={reply()} fallback={<p class="count">{props.run?.summary}</p>}>
                    {(text) => <Markdown text={text()} />}
                  </Show>
                }
              >
                {(content) => <Markdown text={(content() as { text: string }).text} />}
              </Show>
            </article>
          </Match>
        </Switch>
      </div>
    </section>
  );
}

function Steps(props: { entries: Entry[]; asking?: boolean }) {
  return (
    <ol class="run-steps" data-tone={props.asking ? 'asking' : undefined}>
      <For each={steps(props.entries)}>
        {(step) => (
          <li data-status={step.status}>
            <span class="run-step-mark" aria-hidden="true">
              {step.status === 'done' ? '✓' : step.status === 'failed' ? '✗' : '›'}
            </span>
            <span>{step.text}</span>
          </li>
        )}
      </For>
    </ol>
  );
}
