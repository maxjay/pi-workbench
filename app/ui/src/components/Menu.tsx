// A button that opens a short list of things to do.

import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';

export interface Choice {
  /** A thin line between groups of choices, and nothing to pick. */
  rule?: boolean;
  label: string;
  detail?: string;
  /** What the choice says once clicked, when it must be clicked twice to be done. */
  sure?: string;
  run: () => unknown;
}

/** A thin line between groups of choices. */
export const RULE: Choice = { rule: true, label: '', run: () => undefined };

export function Menu(props: {
  label: string;
  /** Show the label as words. Without it the button is three dots. */
  words?: boolean;
  /** Only an arrow: it is the second half of a button beside it. */
  arrow?: boolean;
  choices: Choice[];
  busy?: boolean;
  classList?: Record<string, boolean>;
}) {
  const [open, setOpen] = createSignal(false);
  const [armed, setArmed] = createSignal<string>();
  let box!: HTMLDivElement;
  const away = (e: PointerEvent) => {
    if (!box.contains(e.target as Node)) setOpen(false);
  };
  createEffect(() => {
    if (!open()) return setArmed();
    document.addEventListener('pointerdown', away);
    onCleanup(() => document.removeEventListener('pointerdown', away));
  });
  return (
    <div class="menu" ref={box}>
      <button
        type="button"
        class="button"
        classList={props.classList}
        aria-label={props.words ? undefined : props.label}
        aria-expanded={open()}
        disabled={props.busy}
        onClick={() => setOpen(!open())}
      >
        {props.arrow ? '▾' : props.words ? `${props.label} ▾` : '···'}
      </button>
      <Show when={open()}>
        <div class="menu-list">
          <For each={props.choices}>
            {(choice) => (
              <Show when={!choice.rule} fallback={<span class="menu-rule" />}>
                <button
                  type="button"
                  class="row menu-choice"
                  onClick={() => {
                    if (choice.sure && armed() !== choice.label) return setArmed(choice.label);
                    setOpen(false);
                    void choice.run();
                  }}
                >
                  <span classList={{ 'note-failed': armed() === choice.label }}>
                    {armed() === choice.label ? choice.sure : choice.label}
                  </span>
                  <Show when={choice.detail && armed() !== choice.label}>
                    <span class="dim">{choice.detail}</span>
                  </Show>
                </button>
              </Show>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
