// Ctrl K: type to filter the commands, move with the arrows, Enter runs
// one, Escape closes. What the commands are lives in state/commands.

import { createSignal, For, Show } from 'solid-js';
import { type Command, search } from '../state/commands';

const [isOpen, setOpen] = createSignal(false);
const [query, setQuery] = createSignal('');
const [selected, setSelected] = createSignal(0);

export function openPalette() {
  setQuery('');
  setSelected(0);
  setOpen(true);
}

export function closePalette() {
  setOpen(false);
}

export function Palette() {
  const results = () => search(query());
  const run = (command: Command | undefined) => {
    if (!command) return;
    closePalette();
    command.run();
  };
  const onKey = (e: KeyboardEvent) => {
    const count = results().length;
    if (e.key === 'ArrowDown') setSelected((selected() + 1) % Math.max(count, 1));
    else if (e.key === 'ArrowUp') setSelected((selected() - 1 + count) % Math.max(count, 1));
    else if (e.key === 'Enter') run(results()[selected()]);
    else if (e.key === 'Escape') closePalette();
    else return;
    e.preventDefault();
  };
  return (
    <Show when={isOpen()}>
      <button type="button" class="scrim" aria-label="Close" tabIndex={-1} onClick={closePalette} />
      <div class="palette" role="dialog" aria-label="Go to anything">
        <input
          class="palette-input"
          ref={(el) => queueMicrotask(() => el.focus())}
          placeholder="Go to a tab or a setting, or run an action…"
          value={query()}
          onInput={(e) => {
            setQuery(e.currentTarget.value);
            setSelected(0);
          }}
          onKeyDown={onKey}
        />
        <div class="palette-results" role="listbox" aria-label="Results">
          <Show
            when={results().length > 0}
            fallback={<p class="palette-empty">Nothing matches “{query()}”</p>}
          >
            <For each={results()}>
              {(command, i) => (
                <>
                  <Show when={i() === 0 || results()[i() - 1].group !== command.group}>
                    <p class="palette-group" role="presentation">
                      {command.group}
                    </p>
                  </Show>
                  <button
                    type="button"
                    role="option"
                    class="palette-item"
                    aria-selected={i() === selected()}
                    onMouseEnter={() => setSelected(i())}
                    onClick={() => run(command)}
                  >
                    <span>{command.title}</span>
                    <span class="palette-detail">{command.detail}</span>
                  </button>
                </>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
}
