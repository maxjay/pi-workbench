// The Pull request list on the left: this folder's branch and its pull
// request. Before there is one: the commits and work items it would hold,
// and the way to open it. Once there is one: the way to its own tab,
// where it is reviewed.

import { createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { linkItems, pullBefore, pullStatus, written } from '../../state/pull';

export function Pull(props: {
  dir: string;
  /** Opens the form for a new pull request. */
  onNew: () => void;
  /** Opens a pull request's own tab. */
  onOpen: (pr: number) => void;
}) {
  const [view, { refetch }] = createResource(
    () => [props.dir, written()] as const,
    ([dir]) => pullStatus(dir, null),
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
                  <span class="dim">
                    {open()} open {open() === 1 ? 'thread' : 'threads'}
                    <Show when={v().runs[0]}>
                      {(run) => (
                        <>
                          {' '}
                          · {run().name}{' '}
                          <span class="pull-check" data-result={run().result ?? 'none'}>
                            {run().result ?? run().status}
                          </span>
                        </>
                      )}
                    </Show>
                  </span>
                </div>
                <button
                  type="button"
                  class="button button-primary pull-new"
                  onClick={() => props.onOpen(pr().id)}
                >
                  Review it
                </button>
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
