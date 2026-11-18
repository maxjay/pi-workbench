// The Pull request list on the left: the branch's pull request as parts.
// Its work items, its pipeline runs, its review threads. Picking a run
// opens its log in the middle, and picking a thread opens the thread.

import { createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { openUrl } from '../../components/open';
import { linkItems, pullBefore, pullStatus, written } from '../../state/pull';

export function Pull(props: {
  dir: string;
  /** The pull request asked for from Home. Undefined for the branch's own. */
  pr: number | undefined;
  /** The middle tab shown, to mark the row it came from. */
  current: string | undefined;
  /** Opens the form for a new pull request. */
  onNew: () => void;
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
              <Before dir={props.dir} branch={v().branch} items={v().workItems} onNew={props.onNew} />
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

/** A branch with no pull request yet: what one would hold, and the way to open it. */
function Before(props: {
  dir: string;
  branch: string | null;
  items: { id: number; title: string | null; state: string | null }[];
  onNew: () => void;
}) {
  const [before] = createResource(
    () => [props.dir, props.branch, written()] as const,
    ([dir]) => pullBefore(dir),
  );
  const [ids, setIds] = createSignal('');
  const linking = createAction(async () => {
    const numbers = ids()
      .split(/[^0-9]+/)
      .filter(Boolean)
      .map(Number);
    if (numbers.length === 0) return;
    await linkItems(props.dir, numbers);
    setIds('');
  });
  const commits = () => before.latest?.commits ?? [];

  return (
    <>
      <div class="pull-part">
        <span class="dim">
          {props.branch ?? 'no branch'}
          <Show when={before.latest}>{(b) => <> · into {b().base.replace(/^origin\//, '')}</>}</Show>
        </span>
        <h1 class="pull-name">No pull request yet</h1>
      </div>

      <section class="pull-part" aria-labelledby="before-items">
        <h2 id="before-items" class="pull-label">
          Work items
        </h2>
        <For each={props.items} fallback={<span class="dim">None linked to this branch.</span>}>
          {(item) => (
            <div class="row pull-line">
              <span class="dim">#{item.id}</span>
              <span class="tree-name">{item.title}</span>
              <span class="dim">{item.state}</span>
            </div>
          )}
        </For>
        <form
          class="pull-link"
          onSubmit={(e) => {
            e.preventDefault();
            void linking.run();
          }}
        >
          <input
            class="ask-input"
            placeholder="Link a work item by its number"
            aria-label="Work item numbers to link"
            value={ids()}
            onInput={(e) => setIds(e.currentTarget.value)}
          />
          <button type="submit" class="button" disabled={linking.busy() || !/[0-9]/.test(ids())}>
            Link
          </button>
        </form>
        <Show when={linking.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
      </section>

      <section class="pull-part" aria-labelledby="before-commits">
        <h2 id="before-commits" class="pull-label">
          Commits · {commits().length} ahead
        </h2>
        <Show when={before.error}>
          <p class="note-failed">{String(before.error?.message ?? before.error)}</p>
        </Show>
        <For each={commits()} fallback={<span class="dim">Nothing to open a pull request for yet.</span>}>
          {(commit) => (
            <div class="row pull-line">
              <span class="dim">{commit.sha.slice(0, 7)}</span>
              <span class="tree-name">{commit.subject}</span>
            </div>
          )}
        </For>
      </section>

      <Show when={commits().length > 0}>
        <button type="button" class="button button-primary pull-new" onClick={props.onNew}>
          New pull request
        </button>
      </Show>
    </>
  );
}
