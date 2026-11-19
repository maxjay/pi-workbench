// Home: the ask line, which starts a scratchpad, then your pull requests,
// then your workspaces.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../components/action';
import { Glyph } from '../components/Glyph';
import { openFolderDialog } from '../components/OpenFolder';
import { ask, connect, status, stopReason } from '../daemon';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { agentName, defaultAgent, start } from '../state/conversations';
import { open } from '../state/tabs';
import { refreshWorkspaces, workspaces } from '../state/workspaces';
import { Pulls } from './home/Pulls';
import { Tickets } from './home/Tickets';
import { setConversation } from './workspace/ui';

/** What a workspace is on: its branch, or why it has none. */
function standing(view: WorkspaceView) {
  if (!view.exists) return 'not there any more';
  if (view.workspace.scratch) return 'scratchpad';
  return view.branch ?? 'not a git folder';
}

/** A folder's path with the home folder as `~`. */
const short = (path: string) => path.replace(/^\/home\/[^/]+/, '~');

export function Home() {
  const [text, setText] = createSignal('');
  // What you ask becomes a scratchpad: an empty folder of its own, opened
  // like any workspace, with the conversation already started in it.
  const asking = createAction(async () => {
    const asked = text().trim();
    const view = await ask({ method: 'scratch.new', params: { text: asked } }, 'workspace');
    const conversation = await start(defaultAgent(), view.workspace.path, asked);
    await refreshWorkspaces();
    setConversation(view.workspace.id, conversation);
    setText('');
    open(`ws:${view.workspace.id}`);
  });
  const ready = () => status() === 'ready' && !asking.busy() && text().trim() !== '';

  return (
    <div class="home">
      <Show when={status() === 'down'}>
        <p class="home-down" role="alert">
          <span class="note-failed">{stopReason() ?? 'wbd stopped'}</span>
          <button type="button" class="button" onClick={() => void connect()}>
            Start it again
          </button>
        </p>
      </Show>

      <form
        class="ask-line"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready()) void asking.run();
        }}
      >
        <div class="ask-line-box">
          <label for="ask-line">/</label>
          <input
            id="ask-line"
            placeholder={`Ask ${agentName[defaultAgent()]} anything`}
            autocomplete="off"
            value={text()}
            disabled={asking.busy()}
            onInput={(e) => setText(e.currentTarget.value)}
          />
          <button type="submit" class="button button-primary" disabled={!ready()}>
            Start
          </button>
        </div>
        <Show when={asking.error()} fallback={<p class="dim">Starts a scratchpad: a folder of its own.</p>}>
          {(error) => <p class="note-failed">{error()}</p>}
        </Show>
      </form>

      <Tickets />
      <Pulls />

      <section class="section" aria-labelledby="workspaces">
        <div class="section-head">
          <h2 id="workspaces">Workspaces</h2>
          <span class="top-gap" />
          <button type="button" class="button" disabled={status() !== 'ready'} onClick={openFolderDialog}>
            Open folder
          </button>
        </div>
        <div class="rows">
          <For
            each={workspaces()}
            fallback={<p class="rows-none dim">None yet. Open a folder in WSL to start one.</p>}
          >
            {(view) => (
              <button type="button" class="row table-row" onClick={() => open(`ws:${view.workspace.id}`)}>
                <Glyph name={view.workspace.name} size={12} />
                <span class="table-name" title={view.workspace.path}>
                  {view.workspace.name}
                  <Show when={!view.workspace.scratch}>
                    <span class="table-aside">{short(view.workspace.path)}</span>
                  </Show>
                </span>
                <span class="dim">{standing(view)}</span>
                <span />
              </button>
            )}
          </For>
        </div>
      </section>
    </div>
  );
}
