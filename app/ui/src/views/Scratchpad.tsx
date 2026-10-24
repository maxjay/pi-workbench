// The Scratchpad tab: ad-hoc asks and goals, each a Pi session in a folder
// of its own. The list on the left, the conversation in the middle, and
// the files Pi wrote beside it when you want them.

import { createSignal, Show } from 'solid-js';
import { createAction } from '../components/action';
import { ErrorNote } from '../components/Note';
import { live, selected, startScratchpad } from '../state/sessions';
import { Conversation } from './scratch/Conversation';
import { Nav } from './scratch/Nav';
import { Viewer } from './scratch/Viewer';

const [filesOpen, setFilesOpen] = createSignal(false);

export function Scratchpad() {
  const session = () => {
    const id = selected();
    return id ? live[id] : undefined;
  };
  return (
    <div class="page scratchpad">
      <Nav />
      <Show when={session()} fallback={<Start />}>
        {(session) => (
          <>
            <Conversation
              session={session()}
              filesOpen={filesOpen()}
              onFiles={() => setFilesOpen(!filesOpen())}
            />
            <Show when={filesOpen()}>
              <Viewer session={session()} onClose={() => setFilesOpen(false)} />
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}

/** Nothing picked yet: ask something to start. */
function Start() {
  const [text, setText] = createSignal('');
  const start = createAction(() => startScratchpad(text().trim()));
  return (
    <main class="conversation conversation-start">
      <h1>What should Pi do?</h1>
      <textarea
        class="ask-box"
        rows={3}
        placeholder="An ask or a goal. Pi gets a folder of its own for it."
        value={text()}
        disabled={start.busy()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && text().trim()) {
            e.preventDefault();
            void start.run();
          }
        }}
      />
      <ErrorNote error={start.error()} />
    </main>
  );
}
