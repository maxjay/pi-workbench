// The scratchpads down the left: a box to start one, then the active ones
// and the done ones, newest first. Each says where it stands.

import { createSignal, For } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import { isShown, type Scratchpad, scratchpads, showScratchpad, startScratchpad } from '../../state/sessions';

export function when(updated: number) {
  const minutes = Math.round((Date.now() - updated) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 24 * 60) return `${Math.round(minutes / 60)} h ago`;
  return new Date(updated).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

/** Where a scratchpad stands, as its dot shows it. */
export function standing(pad: Pick<Scratchpad, 'busy' | 'asking' | 'stopped' | 'done'>) {
  if (pad.asking) return 'asking';
  if (pad.busy) return 'busy';
  if (pad.stopped) return 'stopped';
  if (pad.done) return 'done';
  return 'idle';
}

function subtitle(pad: Scratchpad) {
  if (pad.asking) return 'Waiting for your answer';
  if (pad.busy) return 'Pi is working';
  if (pad.stopped) return 'Pi stopped';
  return when(pad.updated);
}

export function Nav() {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    const ask = text().trim();
    if (!ask) return;
    await startScratchpad(ask);
    setText('');
  });
  const show = createAction(showScratchpad);
  const groups = () =>
    [
      { title: 'Active', pads: scratchpads().filter((pad) => !pad.done) },
      { title: 'Done', pads: scratchpads().filter((pad) => pad.done) },
    ].filter((group) => group.pads.length > 0);

  return (
    <nav class="scratch-nav" aria-label="Scratchpads">
      <label class="new-pad">
        <Icon name="plus" size={14} />
        <span class="visually-hidden">New ask or goal</span>
        <input
          placeholder="New ask or goal…"
          value={text()}
          disabled={start.busy()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              void start.run();
            }
          }}
        />
      </label>
      <ErrorNote error={start.error() ?? show.error()} />
      <For each={groups()}>
        {(group) => (
          <div class="pad-group">
            <p class="pad-group-title">{group.title}</p>
            <For each={group.pads}>
              {(pad) => (
                <button
                  type="button"
                  class="pad"
                  aria-current={isShown(pad) ? 'true' : undefined}
                  onClick={() => void show.run(pad)}
                >
                  <span class="pad-dot" data-state={standing(pad)} />
                  <span class="pad-text">
                    <span class="pad-title">{pad.title}</span>
                    <span class="pad-when">{subtitle(pad)}</span>
                  </span>
                </button>
              )}
            </For>
          </div>
        )}
      </For>
    </nav>
  );
}
