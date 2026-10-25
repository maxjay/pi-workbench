// The Scratchpad tab: ad-hoc asks and goals, each a Pi session in a folder
// of its own. The list on the left, the conversation in the middle, and
// the files Pi wrote beside it when you want them.

import { createEffect, createSignal, Show } from 'solid-js';
import { createAction } from '../components/action';
import { ErrorNote } from '../components/Note';
import { current, saved, scratchpads, showScratchpad, view } from '../state/sessions';
import { Conversation } from './scratch/Conversation';
import { Nav } from './scratch/Nav';
import { Viewer } from './scratch/Viewer';

/** The file the viewer shows, once it's open. */
const [viewing, setViewing] = createSignal<{ path?: string }>();

export function Scratchpad() {
  // Something is always shown once there's something to show: the newest
  // active scratchpad, read from its file.
  const first = createAction(showScratchpad);
  createEffect(() => {
    if (view() || first.busy() || first.error() || saved.loading) return;
    const pad = scratchpads().find((p) => !p.done) ?? scratchpads()[0];
    if (pad) void first.run(pad);
  });

  return (
    <div class="page scratchpad">
      <Nav />
      <Show
        when={current()}
        fallback={
          <main class="conversation conversation-empty">
            <div>
              <Show when={!first.busy() && scratchpads().length === 0}>
                <h1>Nothing in the scratchpad yet</h1>
                <p>
                  Write an ask or a goal in the box on the left, or on Home. Each one is a Pi session of its
                  own, with your instructions, MCP servers and models, and it stays here until you mark it
                  done.
                </p>
              </Show>
              <ErrorNote error={first.error()} />
            </div>
          </main>
        }
      >
        {(session) => (
          <>
            <Conversation
              session={session()}
              filesOpen={viewing() !== undefined}
              onFiles={() => setViewing(viewing() ? undefined : {})}
              onOpenFile={(path) => setViewing({ path })}
            />
            <Show when={viewing()}>
              {(shown) => <Viewer session={session()} path={shown().path} onClose={() => setViewing()} />}
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
