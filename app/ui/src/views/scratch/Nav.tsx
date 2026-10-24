// The scratchpads down the left: a box to start one, then the ones Pi is
// working in, today's, and earlier ones, newest first.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import {
  type Scratchpad,
  scratchpads,
  selected,
  showScratchpad,
  startScratchpad,
} from '../../state/sessions';

const day = 24 * 60 * 60 * 1000;

function when(updated: number) {
  const minutes = Math.round((Date.now() - updated) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 24 * 60) return `${Math.round(minutes / 60)} h ago`;
  return new Date(updated).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

function groups(pads: Scratchpad[]) {
  const today = new Date().setHours(0, 0, 0, 0);
  const list = [
    { title: 'Working', pads: pads.filter((pad) => pad.busy) },
    { title: 'Today', pads: pads.filter((pad) => !pad.busy && pad.updated >= today) },
    {
      title: 'Yesterday',
      pads: pads.filter((pad) => !pad.busy && pad.updated < today && pad.updated >= today - day),
    },
    { title: 'Earlier', pads: pads.filter((pad) => !pad.busy && pad.updated < today - day) },
  ];
  return list.filter((group) => group.pads.length > 0);
}

export function Nav() {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    await startScratchpad(text().trim() || undefined);
    setText('');
  });
  const show = createAction(showScratchpad);

  return (
    <nav class="scratch-nav" aria-label="Scratchpads">
      <label class="new-pad">
        <Icon name="plus" size={14} />
        <span class="visually-hidden">New scratchpad</span>
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
      <For each={groups(scratchpads())}>
        {(group) => (
          <div class="pad-group">
            <p class="pad-group-title">{group.title}</p>
            <For each={group.pads}>
              {(pad) => (
                <button
                  type="button"
                  class="pad"
                  aria-current={pad.id !== undefined && pad.id === selected() ? 'true' : undefined}
                  onClick={() => void show.run(pad)}
                >
                  <span class="pad-dot" data-state={pad.busy ? 'busy' : pad.id ? 'live' : 'saved'} />
                  <span class="pad-text">
                    <span class="pad-title">{pad.title}</span>
                    <span class="pad-when">{pad.busy ? 'Pi is working' : when(pad.updated)}</span>
                  </span>
                </button>
              )}
            </For>
          </div>
        )}
      </For>
      <Show when={scratchpads().length === 0}>
        <p class="pad-empty">Each ask or goal is a Pi session in a folder of its own.</p>
      </Show>
    </nav>
  );
}
