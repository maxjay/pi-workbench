// The Pull request list on the left: the branch's pull request as parts.
// Its work items, its pipeline runs, its review threads. Picking a run
// opens its log in the middle, and picking a thread opens the thread.

import { createResource, For, onCleanup, onMount, Show } from 'solid-js';
import { openUrl } from '../../components/open';
import { pullStatus, written } from '../../state/pull';

export function Pull(props: {
  dir: string;
  /** The pull request asked for from Home. Undefined for the branch's own. */
  pr: number | undefined;
  /** The middle tab shown, to mark the row it came from. */
  current: string | undefined;
  onLog: (run: number) => void;
  onThread: (pr: number, thread: number) => void;
}) {
  const [view, { refetch }] = createResource(
    () => [props.dir, props.pr ?? null, written()] as const,
    ([dir, pr]) => pullStatus(dir, pr),
  );
  // Asked again every half minute while this list is shown.
  onMount(() => {
    const timer = setInterval(() => document.hidden || void refetch(), 30_000);
    onCleanup(() => clearInterval(timer));
  });
  const open = () => view.latest?.threads.filter((thread) => thread.unresolved).length ?? 0;

  return (
    <div class="pull-view">
      <Show when={view.error}>
        <p class="left-note note-failed">{String(view.error?.message ?? view.error)}</p>
      </Show>
      <Show
        when={view.latest}
        fallback={
          <Show when={view.loading}>
            <p class="left-note dim">Asking dova</p>
          </Show>
        }
      >
        {(v) => (
          <Show
            when={v().pr}
            fallback={
              <div class="pull-part">
                <span class="dim">{v().branch ?? 'no branch'}</span>
                <h1 class="pull-name">No pull request yet</h1>
              </div>
            }
          >
            {(pr) => (
              <>
                <div class="pull-part">
                  <span class="dim">
                    #{pr().id} · {pr().isDraft ? 'draft' : pr().status} · into {pr().targetBranch}
                  </span>
                  <h1 class="pull-name">{pr().title}</h1>
                  <Show when={v().branch !== pr().sourceBranch}>
                    <p class="pull-note">
                      It is on {pr().sourceBranch}. This folder is on {v().branch ?? 'no branch'}.
                    </p>
                  </Show>
                </div>

                <section class="pull-part" aria-labelledby="pull-items">
                  <h2 id="pull-items" class="pull-label">
                    Work items
                  </h2>
                  <For each={v().workItems} fallback={<span class="dim">None linked.</span>}>
                    {(item) => (
                      <button
                        type="button"
                        class="row pull-line"
                        title="Opens in your browser"
                        onClick={() => openUrl(`${pr().url.split('/_git/')[0]}/_workitems/edit/${item.id}`)}
                      >
                        <span class="dim">#{item.id}</span>
                        <span class="tree-name">{item.title}</span>
                        <span class="dim">{item.state}</span>
                      </button>
                    )}
                  </For>
                </section>

                <section class="pull-part" aria-labelledby="pull-runs">
                  <h2 id="pull-runs" class="pull-label">
                    Pipeline runs
                  </h2>
                  <For each={v().runs} fallback={<span class="dim">None yet.</span>}>
                    {(run) => (
                      <button
                        type="button"
                        class="row pull-line"
                        aria-current={props.current === `log:${run.id}` ? 'true' : undefined}
                        onClick={() => props.onLog(run.id)}
                      >
                        <span>{run.name}</span>
                        <span class="tree-name">
                          <span class="pull-check" data-result={run.result ?? 'none'}>
                            {run.result ?? run.status}
                          </span>
                          <span class="dim"> · run {run.id}</span>
                        </span>
                      </button>
                    )}
                  </For>
                </section>

                <section class="pull-part" aria-labelledby="pull-threads">
                  <h2 id="pull-threads" class="pull-label">
                    Threads · {open()} open, {v().threads.length - open()} resolved
                  </h2>
                  <For each={v().threads} fallback={<span class="dim">None.</span>}>
                    {(thread) => (
                      <button
                        type="button"
                        class="row pull-thread-row"
                        aria-current={props.current === `thread:${pr().id}:${thread.id}` ? 'true' : undefined}
                        onClick={() => props.onThread(pr().id, thread.id)}
                      >
                        <span classList={{ dim: !thread.unresolved }}>
                          <span classList={{ 'pull-thread': thread.unresolved }}>
                            #{thread.id} {thread.unresolved ? 'open' : 'resolved'}
                          </span>
                          <span class="dim">
                            {' '}
                            ·{' '}
                            {thread.location
                              ? `${thread.location.file}:${thread.location.line ?? ''}`
                              : 'general'}{' '}
                            · {thread.lastAuthor}
                          </span>
                        </span>
                        <span classList={{ dim: !thread.unresolved }}>{thread.lastComment}</span>
                      </button>
                    )}
                  </For>
                </section>
              </>
            )}
          </Show>
        )}
      </Show>
    </div>
  );
}
