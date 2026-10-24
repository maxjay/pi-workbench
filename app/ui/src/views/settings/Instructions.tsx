// AGENTS.md: what Pi is told at the start of every session.

import { createEffect, createSignal, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import { instructions, saveInstructions } from '../../state/settings';

export function Instructions() {
  // The file as last read, kept while it's read again. (An errored
  // resource throws when read, hence the check.)
  const file = () => (instructions.error ? undefined : instructions.latest);
  const [draft, setDraft] = createSignal('');
  createEffect(() => setDraft(file()?.text ?? ''));
  const save = createAction(() => saveInstructions(draft()));
  const changed = () => file() !== undefined && draft() !== file()?.text;

  return (
    <section class="settings-section">
      <header class="section-head">
        <div>
          <h1>Instructions</h1>
          <p class="section-sub">What Pi is told at the start of every session, in every codespace.</p>
        </div>
        <button
          type="button"
          class="button button-primary"
          disabled={!changed() || save.busy()}
          onClick={() => void save.run()}
        >
          {save.busy() ? 'Saving…' : 'Save'}
        </button>
      </header>
      <ErrorNote error={instructions.error ?? save.error()} />
      <Show when={file()?.exists === false}>
        <p class="note">There’s no AGENTS.md yet. Saving creates it.</p>
      </Show>
      <textarea
        class="editor"
        spellcheck={false}
        value={draft()}
        disabled={!file()}
        onInput={(e) => setDraft(e.currentTarget.value)}
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key === 's') {
            e.preventDefault();
            if (changed()) void save.run();
          }
        }}
      />
      <p class="section-path mono">{file()?.path}</p>
    </section>
  );
}
