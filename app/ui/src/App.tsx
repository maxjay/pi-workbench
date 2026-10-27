// The window: tabs on top, the toolbar under them, the active tab's page,
// and Ctrl K over everything.

import { lazy, Match, onCleanup, onMount, Switch } from 'solid-js';
import { OpenFolder } from './components/OpenFolder';
import { closePalette, openPalette, Palette } from './components/Palette';
import { TabStrip } from './components/TabStrip';
import { Toolbar } from './components/Toolbar';
import './state/app';
import { webTab } from './state/browser';
import { active, close, open, spaceOf, taskOf } from './state/tabs';
import { Home } from './views/Home';
import { Scratchpad } from './views/Scratchpad';
import { Settings } from './views/Settings';
import { NewTask, TaskPage } from './views/Task';
import { WebPage } from './views/Web';

// The editor and the terminals are big; they load with the first codespace.
const SpacePage = lazy(() => import('./views/Space').then((m) => ({ default: m.SpacePage })));

/** Keys a shell needs, such as Ctrl W to delete a word, which go to a terminal rather than the app. */
const shellKeys = new Set(['k', 'w', 't', 'n']);

function onKey(e: KeyboardEvent) {
  const ctrl = e.ctrlKey || e.metaKey;
  if (ctrl && shellKeys.has(e.key) && (e.target as Element | null)?.closest?.('.xterm')) return;
  if (ctrl && e.key === 'k') openPalette();
  else if (ctrl && e.key === 'w') close(active());
  else if (ctrl && e.key === 't') open('home');
  else if (ctrl && e.key === ',') open('settings');
  else if (ctrl && e.key === 'n') open('scratch');
  else if (e.key === 'Escape') closePalette();
  else return;
  e.preventDefault();
}

export function App() {
  onMount(() => window.addEventListener('keydown', onKey));
  onCleanup(() => window.removeEventListener('keydown', onKey));
  return (
    <div class="window">
      <TabStrip />
      <Toolbar />
      <main class="content">
        <Switch>
          <Match when={active() === 'home'}>
            <Home />
          </Match>
          <Match when={active() === 'scratch'}>
            <Scratchpad />
          </Match>
          <Match when={active() === 'settings'}>
            <Settings />
          </Match>
          <Match when={active() === 'newtask'}>
            <NewTask />
          </Match>
          <Match when={taskOf(active())} keyed>
            {(id) => <TaskPage id={id} />}
          </Match>
          <Match when={spaceOf(active())} keyed>
            {(id) => <SpacePage id={id} />}
          </Match>
          <Match when={webTab(active())} keyed>
            {(id) => <WebPage id={id} />}
          </Match>
        </Switch>
      </main>
      <Palette />
      <OpenFolder />
    </div>
  );
}
