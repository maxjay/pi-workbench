// A pull request's checks, in the middle: the runs on it. The picked run
// is open under its row: its steps, and what the picked step printed. A
// file and line in a log that this pull request changes opens in the diff.
// Picking a run also tells the chat on the right that you are on it.

import { createEffect, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { openUrl } from '../../components/open';
import type { CheckRun } from '../../protocol/CheckRun';
import type { Step } from '../../protocol/Step';
import { rerun, runSteps, stepLog } from '../../state/review';
import { type Pull, took, when } from './parts';

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
    result === 'succeeded' ? 'ok' : result === 'failed' ? 'failed' : result === null ? 'working' : 'faint';
  return { words, state };
}

export function Checks(props: {
  p: Pull;
  runs: CheckRun[] | undefined;
  /** Asks for the runs again. */
  onChanged: () => void;
  /** Whether the pull request's tab is the one shown. */
  shown: () => boolean;
}) {
  const p = () => props.p;
  const [picked, setPicked] = createSignal<number>();
  const run = () => props.runs?.find((r) => r.id === picked()) ?? props.runs?.[0];
  const [steps, { refetch }] = createResource(
    () => run()?.id,
    (id) => runSteps(p().dir, id),
  );
  // A run that isn't finished is asked about again while this is shown.
  createEffect(() => {
    if (!run() || run()?.status === 'completed') return;
    const timer = setInterval(() => {
      if (document.hidden || !props.shown()) return;
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
    ([id, number]) => stepLog(p().dir, id, number),
  );
  const errors = () => [
    ...(steps.latest?.errors ?? []),
    ...(steps.latest?.steps.flatMap((s) => s.errors) ?? []),
  ];

  const again = createAction(async () => {
    const r = run();
    if (!r) return;
    await rerun(p().dir, r.id);
    props.onChanged();
    void refetch();
  });
  /** What the chat is told about a run: what it came to, what the pipeline says, and the end of the open step's log. */
  const contextOf = (r: CheckRun) => {
    const [s, tail] = [step(), (log.latest ?? '').split('\n').slice(-80).join('\n')];
    const same = r.id === run()?.id;
    const said =
      same && errors().length
        ? `\nThe pipeline says:\n${errors()
            .map((e) => `- ${e}`)
            .join('\n')}`
        : '';
    const end =
      same && s && tail ? `\nThe end of the "${s.name}" step's log:\n\n\`\`\`\n${tail}\n\`\`\`` : '';
    return `Pipeline run ${r.id} (${r.name} ${r.number}) on this pull request: ${r.result ?? r.status}.${said}${end}`;
  };
  const tell = (r: CheckRun) => p().setAbout({ kind: 'run', run: r.id, name: r.name, context: contextOf(r) });
  // What the chat is told keeps up with the log as it is read.
  createEffect(() => {
    const [about, r] = [p().about(), run()];
    if (about?.kind !== 'run' || !r || about.run !== r.id) return;
    if (about.context !== contextOf(r)) tell(r);
  });
  const changedFiles = () => p().review.files.map((f) => f.path);

  return (
    <div class="checks">
      <Show when={props.runs} fallback={<p class="pr-pad dim">Asking Azure DevOps</p>}>
        <For each={props.runs} fallback={<p class="pr-pad dim">No pipeline has run on it.</p>}>
          {(r) => (
            <>
              <button
                type="button"
                class="row checks-run"
                aria-current={run()?.id === r.id ? 'true' : undefined}
                onClick={() => {
                  setPicked(r.id);
                  tell(r);
                }}
              >
                <span class="sq" data-tone={outcome(r.result, r.status).state} />
                <span>{r.name}</span>
                <span class="dim checks-detail">
                  {[`run ${r.id}`, took(r.startedAt, r.finishedAt), r.commit.slice(0, 7), when(r.queuedAt)]
                    .filter(Boolean)
                    .join(' · ')}
                </span>
                <span class="top-gap" />
                <span class="checks-word" data-state={outcome(r.result, r.status).state}>
                  {outcome(r.result, r.status).words}
                </span>
              </button>
              <Show when={run()?.id === r.id}>
                <div class="checks-open">
                  <div class="checks-steps">
                    <For each={steps.latest?.steps}>
                      {(s, at) => (
                        <Show when={!s.job} fallback={<span class="checks-job">{s.name}</span>}>
                          <button
                            type="button"
                            class="quiet checks-step"
                            aria-pressed={stepAt() === at()}
                            disabled={s.log === null}
                            title={took(s.startedAt, s.finishedAt)}
                            onClick={() => setStepAt(at())}
                          >
                            <span class="sq" data-tone={outcome(s.result, s.state).state} />
                            {s.name}
                          </button>
                        </Show>
                      )}
                    </For>
                    <span class="top-gap" />
                    <Show when={r.result === 'failed'}>
                      <button
                        type="button"
                        class="quiet"
                        disabled={again.busy()}
                        onClick={() => void again.run()}
                      >
                        Rerun
                      </button>
                    </Show>
                    <button type="button" class="quiet" onClick={() => openUrl(r.url)}>
                      Open
                    </button>
                  </div>
                  <Show when={steps.error}>
                    <p class="note-failed">
                      {String((steps.error as Error | undefined)?.message ?? steps.error)}
                    </p>
                  </Show>
                  <Show when={again.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
                  <For each={errors()}>{(error) => <p class="note-failed">{error}</p>}</For>
                  <Show when={log.error}>
                    <p class="note-failed">
                      {String((log.error as Error | undefined)?.message ?? log.error)}
                    </p>
                  </Show>
                  <Show
                    when={step()}
                    fallback={
                      <p class="dim">
                        {steps.loading ? 'Reading the run' : 'No step of this run left a log.'}
                      </p>
                    }
                  >
                    <Show when={log.latest !== undefined} fallback={<p class="dim">Reading the log</p>}>
                      <pre class="checks-log">
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
                                      class="link"
                                      onClick={() => p().showFile(part.file, part.line)}
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
              </Show>
            </>
          )}
        </For>
      </Show>
    </div>
  );
}
