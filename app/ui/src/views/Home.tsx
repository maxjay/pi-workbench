// Home: the ask line, which starts a workspace in a repo or a scratchpad,
// then your pull requests, then your workspaces.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../components/action';
import { Glyph } from '../components/Glyph';
import { openFolderDialog } from '../components/OpenFolder';
import { openPalette } from '../components/Palette';
import { ask, connect, status, stopReason } from '../daemon';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { agentName, defaultAgent, start } from '../state/conversations';
import { open } from '../state/tabs';
import { refreshWorkspaces, workspaces } from '../state/workspaces';
import { Pulls } from './home/Pulls';
import { Tickets } from './home/Tickets';
import { setConversation, shortPath } from './workspace/ui';

/** The repos in a workspace, the main one first. */
function reposOf(view: WorkspaceView) {
  const main = view.workspace.main;
  return main ? [main, ...view.repos.filter((name) => name !== main)] : view.repos;
}

/**
 * What a workspace is on. With several repos, how many: pointing at it lists them. One the
 * workbench made is called after its main repo, so that one is not said again. A folder you
 * opened has its branch.
 */
function standing(view: WorkspaceView) {
  if (!view.exists) return 'not there any more';
  if (view.repos.length > 1) return `${view.repos.length} repos`;
  if (view.workspace.main) return '';
  if (view.repos.length === 1) return view.repos[0];
  if (view.workspace.scratch) return 'scratchpad';
  return view.branch ?? 'not a git folder';
}

export function Home() {
  const [text, setText] = createSignal('');
  /** The clone the work starts in, when you picked one. */
  const [repo, setRepo] = createSignal<{ name: string; path: string }>();
  const picking = createAction(async () => {
    const { dir, folders } = await ask({ method: 'repos.list' }, 'folders');
    openPalette(
      [
        { group: '', title: 'No repo', detail: 'a scratchpad', run: () => setRepo() },
        ...folders.map((clone) => ({
          group: 'Repo',
          title: clone.name,
          detail: shortPath(clone.path),
          run: () => setRepo({ name: clone.name, path: clone.path }),
        })),
      ],
      `Start in a repo from ${shortPath(dir)}`,
    );
  });
  // What you ask becomes a workspace: a folder of its own, with the conversation already
  // started in it. With a repo picked, a worktree of it is in the folder from the start, on
  // a branch of its own. Without one it is a scratchpad, and a repo can be put in later.
  const asking = createAction(async () => {
    const asked = text().trim();
    const view = await ask(
      { method: 'scratch.new', params: { text: asked, repo: repo()?.path ?? null } },
      'workspace',
    );
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
          <button
            type="button"
            class="button"
            title="The repo the work starts in"
            disabled={status() !== 'ready' || asking.busy()}
            onClick={() => void picking.run()}
          >
            {repo()?.name ?? 'Repo'}
          </button>
          <button type="submit" class="button button-primary" disabled={!ready()}>
            Start
          </button>
        </div>
        <Show
          when={asking.error() ?? picking.error()}
          fallback={
            <p class="dim">
              <Show
                when={repo()}
                fallback="Starts a scratchpad: a folder of its own. Pick a repo to start in one."
              >
                {(picked) =>
                  asking.busy()
                    ? `Making a worktree of ${picked().name}`
                    : `Starts a workspace: a worktree of ${picked().name}, on a branch of its own.`
                }
              </Show>
            </p>
          }
        >
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
                    <span class="table-aside">{shortPath(view.workspace.path)}</span>
                  </Show>
                </span>
                <span
                  class="dim table-clip"
                  title={view.repos.length > 1 ? reposOf(view).join('\n') : undefined}
                >
                  {standing(view)}
                </span>
                <span />
              </button>
            )}
          </For>
        </div>
      </section>
    </div>
  );
}
