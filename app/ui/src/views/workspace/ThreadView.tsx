// A review thread in the middle: everything said in it, and where you
// answer. Reply, resolve, or both at once.

import { createResource, createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Markdown } from '../../components/Markdown';
import { pullThread, reply, resolve, written } from '../../state/pull';

/** `2026-11-14T19:47:13Z` as a date and time here. */
const when = (iso: string | null) =>
  iso ? new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : '';

export function ThreadView(props: {
  dir: string;
  pr: number;
  thread: number;
  /** Opens the file a thread is on, at its line. */
  onOpenAt: (file: string, line: number | null) => void;
}) {
  const [thread] = createResource(
    () => [props.dir, props.pr, props.thread, written()] as const,
    ([dir, pr, id]) => pullThread(dir, pr, id),
  );
  const [text, setText] = createSignal('');
  const say = createAction(async (andResolve: boolean) => {
    await reply(props.dir, props.pr, props.thread, text().trim(), andResolve);
    setText('');
  });
  const close = createAction(() => resolve(props.dir, props.pr, props.thread));
  const busy = () => say.busy() || close.busy();

  return (
    <div class="thread">
      <Show when={thread.error}>
        <p class="note-failed">{String(thread.error?.message ?? thread.error)}</p>
      </Show>
      <Show when={thread.latest} fallback={<p class="dim">Asking dova</p>}>
        {(t) => (
          <>
            <div class="thread-head">
              <span classList={{ 'pull-thread': t().unresolved, dim: !t().unresolved }}>
                #{t().id} {t().unresolved ? 'open' : 'resolved'}
              </span>
              <Show when={t().location} fallback={<span class="dim">on the whole pull request</span>}>
                {(at) => (
                  <button
                    type="button"
                    class="pull-open"
                    onClick={() => props.onOpenAt(at().file, at().line)}
                  >
                    {at().file}
                    {at().line ? `:${at().line}` : ''}
                  </button>
                )}
              </Show>
            </div>
            <For each={t().comments}>
              {(comment) => (
                <div class="thread-comment">
                  <span class="dim">
                    {comment.author} · {when(comment.publishedAt)}
                  </span>
                  <Markdown text={comment.text} />
                </div>
              )}
            </For>
            <div class="composer">
              <textarea
                class="composer-text"
                rows={3}
                placeholder="Reply"
                aria-label="Reply to this thread"
                value={text()}
                disabled={busy()}
                onInput={(e) => setText(e.currentTarget.value)}
              />
              <div class="composer-foot">
                <Show when={say.error() ?? close.error()}>
                  {(error) => <span class="note-failed">{error()}</span>}
                </Show>
                <span class="top-gap" />
                <Show when={t().unresolved}>
                  <button type="button" class="button" disabled={busy()} onClick={() => void close.run()}>
                    Resolve
                  </button>
                  <button
                    type="button"
                    class="button"
                    disabled={busy() || !text().trim()}
                    onClick={() => void say.run(true)}
                  >
                    Reply and resolve
                  </button>
                </Show>
                <button
                  type="button"
                  class="button button-primary"
                  disabled={busy() || !text().trim()}
                  onClick={() => void say.run(false)}
                >
                  Reply
                </button>
              </div>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
