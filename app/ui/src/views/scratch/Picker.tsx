// The list `@` and `/` open above the box you write in: files under the
// folder Pi works in, or Pi's commands. It narrows as you type; arrows
// move, Enter or Tab puts the pick in, Escape closes it.

import { createEffect, createResource, createSignal, For, on, Show } from 'solid-js';
import type { PiCommand } from '../../protocol/PiCommand';
import type { Session } from '../../protocol/Session';
import { commandsFor, listFiles } from '../../state/sessions';
import { type Choice, picker } from './picker';

/** The last commands Pi listed, for a saved conversation, whose Pi isn't running. */
const [knownCommands, setKnownCommands] = createSignal<PiCommand[]>([]);

export function createPicker(props: {
  session: () => Session;
  text: () => string;
  setText: (text: string) => void;
  box: () => HTMLTextAreaElement | undefined;
}) {
  const [caret, setCaret] = createSignal(0);
  const [closed, setClosed] = createSignal<string>();
  const [index, setIndex] = createSignal(0);
  const before = () => props.text().slice(0, caret());
  const wants = () => {
    if (/^\/\S*$/.test(before())) return 'commands';
    if (/(^|\s)@\S*$/.test(before())) return 'files';
    return undefined;
  };

  // Read afresh each time the list opens, as Pi may have written files since.
  const [opened, setOpened] = createSignal(0);
  createEffect(on(wants, (now, was) => now && now !== was && setOpened(opened() + 1)));
  const [files] = createResource(
    () => (wants() === 'files' ? { dir: props.session().state.cwd, opened: opened() } : undefined),
    async ({ dir }) => (await listFiles(dir)).paths,
  );
  const [commands] = createResource(
    () => (wants() === 'commands' ? { id: props.session().id, opened: opened() } : undefined),
    async ({ id }) => {
      if (!id) return knownCommands();
      const list = await commandsFor(id);
      setKnownCommands(list);
      return list;
    },
  );

  const current = () => {
    if (!wants() || closed() === before()) return undefined;
    return picker(before(), files.latest ?? [], commands.latest ?? knownCommands());
  };
  const at = () => Math.min(index(), Math.max(0, (current()?.choices.length ?? 1) - 1));
  const loading = () => (wants() === 'files' ? files.loading : commands.loading);

  const choose = (choice: Choice) => {
    const shown = current();
    if (!shown) return;
    const start = shown.start;
    const next = before().slice(0, start) + choice.insert + props.text().slice(caret());
    const end = start + choice.insert.length;
    props.setText(next);
    setCaret(end);
    setIndex(0);
    queueMicrotask(() => {
      const box = props.box();
      box?.focus();
      box?.setSelectionRange(end, end);
    });
  };

  /** Follows the caret, which decides what's being typed for. */
  const follow = (box: HTMLTextAreaElement) => {
    setCaret(box.selectionStart);
  };

  /** Handles a key for the list, if it's open. True when it was the list's. */
  const onKey = (e: KeyboardEvent) => {
    const shown = current();
    if (!shown) return false;
    if (e.key === 'Escape') {
      setClosed(before());
      return true;
    }
    const n = shown.choices.length;
    if (n === 0) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      setIndex((at() + (e.key === 'ArrowDown' ? 1 : n - 1)) % n);
      return true;
    }
    if (e.key === 'Enter' || e.key === 'Tab') {
      choose(shown.choices[at()]);
      return true;
    }
    return false;
  };

  const view = () => (
    <Show when={current()}>
      {(shown) => (
        <div
          class="model-menu picker"
          role="listbox"
          aria-label={shown().kind === 'files' ? 'Files' : 'Commands'}
        >
          <div class="model-menu-head">
            <span>{shown().kind === 'files' ? 'Files' : 'Commands'}</span>
            <Show when={shown().kind === 'files'}>
              <span class="count mono picker-where" title={props.session().state.cwd}>
                {/* A left-to-right mark keeps the leading slash where it belongs. */}
                {`\u200e${props.session().state.cwd}`}
              </span>
            </Show>
          </div>
          <For each={shown().choices}>
            {(choice, i) => (
              <button
                type="button"
                role="option"
                class="picker-row"
                aria-selected={i() === at()}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => choose(choice)}
              >
                <span class="picker-main mono">{choice.main}</span>
                <span class="picker-detail">{choice.detail}</span>
                <Show when={choice.tag}>
                  <span class="picker-tag">{choice.tag}</span>
                </Show>
              </button>
            )}
          </For>
          <Show when={shown().choices.length === 0}>
            <p class="model-empty">
              {loading()
                ? 'Looking…'
                : shown().kind === 'files'
                  ? `No file matches ${shown().query}`
                  : `No command called /${shown().query}`}
            </p>
          </Show>
        </div>
      )}
    </Show>
  );

  return { view, onKey, follow, reset: () => setIndex(0) };
}
