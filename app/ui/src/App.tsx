// The window: one row on top, and the page of the tab you are on. Ctrl K
// goes anywhere. Ctrl W closes the tab. Ctrl comma opens Settings.

import { For, onCleanup, onMount, Show } from 'solid-js';
import { OpenFolder, openFolderDialog } from './components/OpenFolder';
import { type Item, openPalette, Palette } from './components/Palette';
import { TopBar, titleOf } from './components/TopBar';
import { active, close, open, pullOf, tabs, workspaceOf } from './state/tabs';
import { workspaces } from './state/workspaces';
import { Home } from './views/Home';
import { ReviewPage } from './views/Review';
import { Settings } from './views/Settings';
import { WorkspacePage } from './views/Workspace';

/** Ctrl K's list: the open tabs, every workspace, and what you can do. */
function go() {
  const items: Item[] = [
    ...tabs().map((tab) => ({ group: 'Open tabs', title: titleOf(tab), run: () => open(tab) })),
    ...(workspaces() ?? []).map((view) => ({
      group: 'Workspaces',
      title: view.workspace.name,
      detail: view.workspace.path,
      run: () => open(`ws:${view.workspace.id}`),
    })),
    { group: 'Do', title: 'Open folder', detail: 'as a workspace', run: openFolderDialog },
    { group: 'Do', title: 'Settings', detail: 'agents, this machine', run: () => open('settings') },
  ];
  openPalette(items, 'Go to anything');
}

function onKey(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey)) return;
  if (e.key === 'k') go();
  else if (e.key === 'w') close(active());
  else if (e.key === ',') open('settings');
  else return;
  e.preventDefault();
}

export function App() {
  onMount(() => window.addEventListener('keydown', onKey));
  onCleanup(() => window.removeEventListener('keydown', onKey));
  return (
    <div class="window">
      <TopBar onGo={go} />
      <main class="content">
        <Show when={active() === 'home'}>
          <Home />
        </Show>
        <Show when={active() === 'settings'}>
          <Settings />
        </Show>
        {/* A workspace's page stays mounted while its tab is open, so its editor keeps its place. */}
        <For each={tabs().filter((tab) => workspaceOf(tab))}>
          {(tab) => (
            <div class="page" hidden={active() !== tab}>
              <WorkspacePage id={workspaceOf(tab) as string} />
            </div>
          )}
        </For>
        {/* So does a pull request's, so its diff and its session keep theirs. */}
        <For each={tabs().filter((tab) => pullOf(tab))}>
          {(tab) => (
            <div class="page" hidden={active() !== tab}>
              <ReviewPage tab={tab} workspace={pullOf(tab)?.workspace ?? ''} pr={pullOf(tab)?.pr ?? 0} />
            </div>
          )}
        </For>
      </main>
      <Palette />
      <OpenFolder />
    </div>
  );
}
