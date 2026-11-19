// A pull request's checks, in the middle: the runs on it, the picked
// run's jobs and their steps, and what the picked step printed. A file and
// line in a log that this pull request changes opens in the diff.

import { createEffect, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { openUrl } from '../../components/open';
import type { CheckRun } from '../../protocol/CheckRun';
import type { Step } from '../../protocol/Step';
import { notesOf, rerun, runSteps, stepLog } from '../../state/review';
import { type Reviewing, took, when } from './parts';
import { NoteBox } from './Threads';

/** `src/auth.ts:42` or `src/auth.ts(42,5)`, anywhere in a line of log. */
const PLACE = /([\w@./\\-]+\.[A-Za-z]\w*)[:(](\d+)/g;

/** A line of log as text and the places in it that are files this pull request changes. */
function parts(line: string, files: string[]) {
  const out: (string | { file: string; line: number; text: string })[] = [];
  let last = 0;
  for (const match of line.matchAll(PLACE)) {
    const named = match[1].replaceAll('\\', '/');
    // A build agent names files by its own folders: the end of the path is what matches.
    const file = files.find((f) => named === f || named.endsWith(`/${f}`));
    if (!file || match.index === undefined) continue;
    out.push(line.slice(last, match.index), { file, line: Number(match[2]), text: match[0] });
    last = match.index + match[0].length;
  }
  out.push(line.slice(last));
  return out;
}

/** What a run or a step came to, and the state it reads as. */
export function outcome(result: string | null, status: string) {
  const words = result ?? (status === 'inProgress' ? 'running' : status === 'completed' ? 'done' : 'waiting');
  const state =
    result === 'succeeded' ? 'ok' : result === 'failed' ? 'failed' : result === null ? 'working' : 'none';
  return { words, state };
}

export function Checks(props: {
  r: Reviewing;
  runs: CheckRun[] | undefined;
  /** Asks for the runs again. */
  onChanged: () => void;
}) {
  const [picked, setPicked] = createSignal<number>();
  const run = () => props.runs?.find((r) => r.id === picked()) ?? props.runs?.[0];
  const [steps, { refetch }] = createResource(
    () => run()?.id,
    (id) => runSteps(props.r.dir, id),
  );
  // A run that isn't finished is asked about again while this is shown.
  createEffect(() => {
    if (run()?.status === 'completed') return;
    const timer = setInterval(() => {
      if (document.hidden) return;
      props.onChanged();
      void refetch();
    }, 10_000);
    onCleanup(() => clearInterval(timer));
  });

  const [stepAt, setStepAt] = createSignal<number>();
  // A run opens on the step that failed, or else on its last step that left a log.
  createEffect(
    on(
      () => steps.latest?.steps,
      (list) => {
        if (!list) return;
        const failed = list.findIndex((s) => !s.job && s.result === 'failed' && s.log !== null);
        const last = list.map((s) => !s.job && s.log !== null).lastIndexOf(true);
        setStepAt(failed >= 0 ? failed : last >= 0 ? last : undefined);
      },
    ),
  );
  const step = (): Step | undefined => steps.latest?.steps[stepAt() ?? -1];
  const [log] = createResource(
    () => {
      const [r, s] = [run(), step()];
      return r && s && s.log !== null ? ([r.id, s.log] as const) : undefined;
    },
    ([id, number]) => stepLog(props.r.dir, id, number),
  );

  const again = createAction(async () => {
    const r = run();
    if (!r) return;
    await rerun(props.r.dir, r.id);
    props.onChanged();
    void refetch();
  });
  const why = () => notesOf(props.r.pull).find((note) => note.run === run()?.id);
  const askWhy = () => {
    const [r, s] = [run(), step()];
    if (!r) return;
    const tail = (log.latest ?? '').split('\n').slice(-80).join('\n');
    const said = [...(steps.latest?.errors ?? []), ...(steps.latest?.steps.flatMap((x) => x.errors) ?? [])];
    props.r.askAgent(
      `Pipeline run ${r.id} (${r.name} ${r.number}) on this pull request ${r.result ?? r.status}. Say why in a few lines, and what would fix it.${
        said.length ? `\n\nThe pipeline says:\n${said.map((e) => `- ${e}`).join('\n')}` : ''
      }${s && tail ? `\n\nThe end of the "${s.name}" step's log:\n\n\`\`\`\n${tail}\n\`\`\`` : ''}`,
      { run: r.id, question: 'Why did this run fail?' },
    );
  };
  const changedFiles = () => props.r.review.files.map((f) => f.path);

  return (
    <div class="checks">
      <div class="checks-list">
        <span class="checks-label">Runs</span>
        <Show when={props.runs} fallback={<p class="checks-note dim">Asking Azure DevOps</p>}>
          <For each={props.runs} fallback={<p class="checks-note dim">No pipeline has run on it.</p>}>
            {(r) => (
              <button
                type="button"
                class="row checks-run"
                aria-current={run()?.id === r.id ? 'true' : undefined}
                onClick={() => setPicked(r.id)}
              >
                <span class="checks-run-top">
                  <span>
                    {r.name} · run {r.id}
                  </span>
                  <span class="mark" data-state={outcome(r.result, r.status).state}>
                    {outcome(r.result, r.status).words}
                  </span>
                </span>
                <span class="dim">
                  {[took(r.startedAt, r.finishedAt), r.commit.slice(0, 7), when(r.queuedAt)]
                    .filter(Boolean)
                    .join(' · ')}
                </span>
              </button>
            )}
          </For>
        </Show>

        <Show when={run()}>
          <span class="checks-label checks-label-next">
            {run()?.name} {run()?.number}
          </span>
          <Show when={steps.error}>
            <p class="checks-note note-failed">{String(steps.error?.message ?? steps.error)}</p>
          </Show>
          <For each={steps.latest?.steps}>
            {(s, at) => (
              <Show
                when={!s.job}
                fallback={
                  <div class="checks-job">
                    <span class="tree-name">{s.name}</span>
                    <span class="mark" data-state={outcome(s.result, s.state).state}>
                      {outcome(s.result, s.state).words}
                    </span>
                  </div>
                }
              >
                <button
                  type="button"
                  class="row checks-step"
                  aria-current={stepAt() === at() ? 'true' : undefined}
                  disabled={s.log === null}
                  onClick={() => setStepAt(at())}
                >
                  <span class="dot" data-state={outcome(s.result, s.state).state} />
                  <span class="tree-name">{s.name}</span>
                  <span class="dim">{took(s.startedAt, s.finishedAt)}</span>
                </button>
              </Show>
            )}
          </For>
        </Show>
      </div>

      <div class="checks-shown">
        <Show when={run()} fallback={<p class="review-pad dim">Nothing to show.</p>}>
          {(r) => (
            <>
              <div class="bar diff-head">
                <span class="tree-name">
                  {r().name} · run {r().id}
                  <Show when={step()}>{(s) => <> › {s().name}</>}</Show>
                </span>
                <Show when={r().result === 'failed'}>
                  <button
                    type="button"
                    class="quiet"
                    disabled={again.busy()}
                    onClick={() => void again.run()}
                  >
                    Rerun
                  </button>
                </Show>
                <button type="button" class="quiet" onClick={() => openUrl(r().url)}>
                  Open
                </button>
              </div>
              <div class="checks-body">
                <Show when={again.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
                <For
                  each={[
                    ...(steps.latest?.errors ?? []),
                    ...(steps.latest?.steps.flatMap((s) => s.errors) ?? []),
                  ]}
                >
                  {(error) => <p class="note-failed">{error}</p>}
                </For>
                <Show when={r().result === 'failed' && !why()}>
                  <div>
                    <button type="button" class="button" onClick={askWhy}>
                      Ask why
                    </button>
                  </div>
                </Show>
                <Show when={why()}>
                  {(note) => <NoteBox note={note()} pull={props.r.pull} label="agent" />}
                </Show>
                <Show when={log.error}>
                  <p class="note-failed">{String(log.error?.message ?? log.error)}</p>
                </Show>
                <Show when={step()} fallback={<p class="dim">No step of this run left a log.</p>}>
                  <Show when={log.latest !== undefined} fallback={<p class="dim">Reading the log</p>}>
                    <pre class="log-text checks-log">
                      <For each={(log.latest ?? '').split(/\r?\n/)}>
                        {(line) => (
                          <span classList={{ 'note-failed': line.startsWith('##[error]') }}>
                            <For each={parts(line, changedFiles())}>
                              {(part) =>
                                typeof part === 'string' ? (
                                  part
                                ) : (
                                  <button
                                    type="button"
                                    class="pull-open"
                                    onClick={() => props.r.showFile(part.file, part.line)}
                                  >
                                    {part.text}
                                  </button>
                                )
                              }
                            </For>
                            {'\n'}
                          </span>
                        )}
                      </For>
                    </pre>
                  </Show>
                </Show>
              </div>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}
