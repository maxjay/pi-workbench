// The window: one row on top, and the page of the tab you are on. Ctrl K
// goes anywhere. Ctrl W closes the tab. Ctrl comma opens Settings.

import { For, onCleanup, onMount, Show } from 'solid-js';
import { OpenFolder, openFolderDialog } from './components/OpenFolder';
import { type Item, openPalette, Palette } from './components/Palette';
import { TopBar, titleOf } from './components/TopBar';
import { codespaces } from './state/codespaces';
import { active, close, codespaceOf, open, pullOf, tabs, ticketOf } from './state/tabs';
import { CodespacePage } from './views/Codespace';
import { Home } from './views/Home';
import { PullPage } from './views/pull/Page';
import { SettingsPage } from './views/Settings';
import { TicketPage } from './views/ticket/Page';
import { LogPage } from './views/WbdLog';

/** Ctrl K's list: the open tabs, every codespace, and what you can do. */
function go() {
  const items: Item[] = [
    ...tabs().map((tab) => ({ group: 'Open tabs', title: titleOf(tab), run: () => open(tab) })),
    ...(codespaces() ?? []).map((view) => ({
      group: 'Codespaces',
      title: view.codespace.name,
      detail: view.codespace.path,
      run: () => open(`cs:${view.codespace.id}`),
    })),
    { group: 'Do', title: 'Open folder', detail: 'as a codespace', run: openFolderDialog },
    { group: 'Do', title: 'Settings', detail: 'agents, this machine', run: () => open('settings') },
    { group: 'Do', title: 'Log', detail: 'what wbd asked and ran, and what failed', run: () => open('log') },
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
      <TopBar />
      <main class="content">
        <Show when={active() === 'home'}>
          <Home />
        </Show>
        <Show when={active() === 'settings'}>
          <SettingsPage />
        </Show>
        <Show when={active() === 'log'}>
          <LogPage />
        </Show>
        {/* A codespace's page stays mounted while its tab is open, so its editor keeps its place. */}
        <For each={tabs().filter((tab) => codespaceOf(tab))}>
          {(tab) => (
            <div class="page" hidden={active() !== tab}>
              <CodespacePage id={codespaceOf(tab) as string} />
            </div>
          )}
        </For>
        {/* So does a pull request's, so its diff and its session keep theirs. */}
        <For each={tabs().filter((tab) => pullOf(tab))}>
          {(tab) => (
            <div class="page" hidden={active() !== tab}>
              <PullPage tab={tab} codespace={pullOf(tab)?.codespace ?? ''} pr={pullOf(tab)?.pr ?? 0} />
            </div>
          )}
        </For>
        <For each={tabs().filter((tab) => ticketOf(tab))}>
          {(tab) => (
            <div class="page" hidden={active() !== tab}>
              <TicketPage tab={tab} codespace={ticketOf(tab)?.codespace ?? ''} id={ticketOf(tab)?.id ?? 0} />
            </div>
          )}
        </For>
      </main>
      <Palette />
      <OpenFolder />
    </div>
  );
}
