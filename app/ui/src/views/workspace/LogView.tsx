// Why a run failed, in the middle: what the pipeline said went wrong, which
// tasks failed, and the first failed task's log. A file and line in the
// log that is in this workspace opens there.

import { createResource, For, Show } from 'solid-js';
import { openUrl } from '../../components/open';
import { pullLog } from '../../state/pull';

/** `src/auth.ts:42` or `src/auth.ts(42,5)`, anywhere in a line of log. */
const PLACE = /([\w@./\\-]+\.[A-Za-z]\w*)[:(](\d+)/g;

/** A line of log as text and the places in it that are files here. */
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

export function LogView(props: {
  dir: string;
  run: number;
  /** The workspace's files, relative to it. */
  files: string[];
  onOpenAt: (file: string, line: number) => void;
  /** Hands the failure to the agent. */
  onAsk: (text: string) => void;
}) {
  const [log] = createResource(
    () => [props.dir, props.run] as const,
    ([dir, run]) => pullLog(dir, run),
  );
  return (
    <div class="log">
      <Show when={log.error}>
        <p class="log-pad note-failed">{String(log.error?.message ?? log.error)}</p>
      </Show>
      <Show when={log.latest} fallback={<p class="log-pad dim">Asking dova</p>}>
        {(l) => (
          <>
            <div class="log-head">
              <span class="tree-name">
                <span class="pull-check" data-result={l().run.result ?? 'none'}>
                  {l().run.result ?? l().run.status}
                </span>
                <span class="dim">
                  {' '}
                  · {l().run.name} · run {l().run.id}
                  <Show when={l().failedTasks.length > 0}> · failed in {l().failedTasks.at(-1)}</Show>
                </span>
              </span>
              <button
                type="button"
                class="button button-primary"
                onClick={() =>
                  props.onAsk(
                    `Pipeline run ${l().run.id} failed. Read why with \`dova pipeline log ${l().run.id}\`, then fix it.`,
                  )
                }
              >
                Ask the agent to fix
              </button>
              <button type="button" class="button" onClick={() => openUrl(l().run.url)}>
                Open in your browser
              </button>
            </div>
            <div class="log-body">
              <For each={l().errors}>{(error) => <p class="note-failed">{error}</p>}</For>
              <For each={l().logs} fallback={<p class="dim">No task left a log.</p>}>
                {(task) => (
                  <pre class="log-text">
                    <For each={task.text.split(/\r?\n/)}>
                      {(line) => (
                        <>
                          <For each={parts(line, props.files)}>
                            {(part) =>
                              typeof part === 'string' ? (
                                part
                              ) : (
                                <button
                                  type="button"
                                  class="pull-open"
                                  onClick={() => props.onOpenAt(part.file, part.line)}
                                >
                                  {part.text}
                                </button>
                              )
                            }
                          </For>
                          {'\n'}
                        </>
                      )}
                    </For>
                  </pre>
                )}
              </For>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
