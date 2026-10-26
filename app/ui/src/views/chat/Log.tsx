// A conversation's entries, as with Pi in a terminal: what you asked, what
// Pi thought, ran and said. It follows the newest line unless you've
// scrolled up to read. Offers Try again when the last thing is an error.

import { For, type JSX, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import type { Session } from '../../protocol/Session';
import { send } from '../../state/sessions';
import { EntryView } from './EntryView';

export function Log(props: {
  session: Session;
  onOpenFile: (path: string) => void;
  /** What an empty conversation says. */
  empty?: JSX.Element;
}) {
  const state = () => props.session.state;
  // Pi has no retry of its own once it gives up: asking again is the way.
  const retry = createAction(() => send(props.session, 'Try again', false));
  const canRetry = (i: number) => {
    const entry = props.session.entries[i];
    return (
      i === props.session.entries.length - 1 &&
      entry.kind === 'notice' &&
      entry.tone === 'error' &&
      !state().busy &&
      !state().ask &&
      !state().stopped
    );
  };

  let log!: HTMLDivElement;
  let content!: HTMLDivElement;
  let following = true;
  onMount(() => {
    const follow = new ResizeObserver(() => {
      if (following) log.scrollTop = log.scrollHeight;
    });
    follow.observe(content);
    onCleanup(() => follow.disconnect());
  });

  return (
    <div
      class="log"
      ref={log}
      onScroll={() => {
        following = log.scrollHeight - log.scrollTop - log.clientHeight < 48;
      }}
    >
      <div class="log-content" ref={content}>
        <For each={props.session.entries}>
          {(entry, i) => (
            <EntryView
              entry={entry}
              cwd={state().cwd}
              last={i() === props.session.entries.length - 1 && state().busy}
              onOpenFile={props.onOpenFile}
              onRetry={canRetry(i()) && !retry.busy() ? () => void retry.run() : undefined}
            />
          )}
        </For>
        <Show when={props.session.entries.length === 0 && !state().busy}>
          <p class="log-empty">{props.empty ?? `Ask Pi anything. It works in ${state().cwd}.`}</p>
        </Show>
        <ErrorNote error={retry.error()} />
      </div>
    </div>
  );
}
