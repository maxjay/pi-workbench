// The list that opens over everything: type to narrow it, Enter to run the
// row picked. Ctrl K opens it with places to go, Ctrl P with a workspace's
// files. Whoever opens it says what is in it.

import { createSignal, For, Show } from 'solid-js';

export interface Item {
  group: string;
  title: string;
  detail?: string;
  run: () => void;
}

const [shown, setShown] = createSignal<{ items: Item[]; hint: string }>();
const [query, setQuery] = createSignal('');
const [picked, setPicked] = createSignal(0);

export function openPalette(items: Item[], hint: string) {
  setQuery('');
  setPicked(0);
  setShown({ items, hint });
}

export const closePalette = () => setShown();

/** Rows are capped, as a list of thousands is no use on screen. */
const MOST = 50;

export function Palette() {
  const rows = () => {
    const words = query().toLowerCase().split(/\s+/).filter(Boolean);
    const all = shown()?.items ?? [];
    const matching = all.filter((item) => {
      const text = `${item.title} ${item.detail ?? ''}`.toLowerCase();
      return words.every((word) => text.includes(word));
    });
    return matching.slice(0, MOST);
  };

  const run = (item: Item | undefined) => {
    if (!item) return;
    closePalette();
    item.run();
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'ArrowDown') setPicked(Math.min(picked() + 1, rows().length - 1));
    else if (e.key === 'ArrowUp') setPicked(Math.max(picked() - 1, 0));
    else if (e.key === 'Enter') run(rows()[picked()]);
    else if (e.key === 'Escape') closePalette();
    else return;
    e.preventDefault();
  };

  return (
    <Show when={shown()}>
      {(palette) => (
        <div class="scrim">
          <button type="button" class="scrim-close" aria-label="Close" onClick={() => closePalette()} />
          <div class="palette" role="dialog" aria-label={palette().hint}>
            <input
              // Typing goes here from the moment it opens, not to whatever is behind it.
              ref={(el) => requestAnimationFrame(() => el.focus())}
              class="palette-input"
              placeholder={palette().hint}
              aria-label={palette().hint}
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                setPicked(0);
              }}
              onKeyDown={onKey}
            />
            <div class="palette-rows">
              <For each={rows()} fallback={<p class="palette-none dim">Nothing matches.</p>}>
                {(item, i) => (
                  <button
                    type="button"
                    class="row palette-row"
                    aria-current={picked() === i() ? 'true' : undefined}
                    onMouseEnter={() => setPicked(i())}
                    onClick={() => run(item)}
                  >
                    <span class="palette-title">{item.title}</span>
                    <span class="palette-detail dim">{item.detail}</span>
                    <span class="palette-group dim">{item.group}</span>
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
