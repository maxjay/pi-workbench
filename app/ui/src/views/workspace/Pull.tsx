// A branch's pull request, as one line in a workspace's Changes: the way
// to its own tab, where it is reviewed. Before there is one, how far the
// branch is ahead and the way to open one.

import { createResource, onCleanup, onMount, Show } from 'solid-js';
import { health } from '../../state/health';
import { pullBefore, pullStatus, written } from '../../state/pull';

export function BranchPull(props: {
  dir: string;
  /** Opens the form for a new pull request. */
  onNew: () => void;
  /** Opens a pull request's own tab. */
  onOpen: (pr: number) => void;
}) {
  // Nothing to ask without dova. A folder that isn't cloned from Azure Repos just has no line.
  const [view, { refetch }] = createResource(
    () => (health()?.dova ? ([props.dir, written()] as const) : undefined),
    ([dir]) => pullStatus(dir, null).catch(() => undefined),
  );
  const [before] = createResource(
    () =>
      view.latest && !view.latest.pr ? ([props.dir, view.latest.branch, written()] as const) : undefined,
    ([dir]) => pullBefore(dir).catch(() => undefined),
  );
  // Asked again every half minute while Changes is shown.
  onMount(() => {
    const timer = setInterval(() => document.hidden || void refetch(), 30_000);
    onCleanup(() => clearInterval(timer));
  });
  const ahead = () => before.latest?.commits.length ?? 0;

  return (
    <Show when={view.latest}>
      {(v) => (
        <Show
          when={v().pr}
          fallback={
            <Show when={ahead() > 0}>
              <div class="branch-pull">
                <span class="dim">
                  {ahead()} {ahead() === 1 ? 'commit' : 'commits'} ahead of{' '}
                  {before.latest?.base.replace(/^origin\//, '')}
                </span>
                <button type="button" class="button" onClick={props.onNew}>
                  New pull request
                </button>
              </div>
            </Show>
          }
        >
          {(pr) => (
            <button
              type="button"
              class="row branch-pull branch-pull-open"
              title="Opens the pull request"
              onClick={() => props.onOpen(pr().id)}
            >
              <span class="accent">#{pr().id}</span>
              <span class="tree-name">{pr().title}</span>
              <span class="dim">{pr().isDraft ? 'draft' : pr().status}</span>
            </button>
          )}
        </Show>
      )}
    </Show>
  );
}
