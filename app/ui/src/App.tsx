// The window: tabs on top, the toolbar under them, the active tab's page,
// and Ctrl K over everything.

import { Match, onCleanup, onMount, Switch } from 'solid-js';
import { closePalette, openPalette, Palette } from './components/Palette';
import { TabStrip } from './components/TabStrip';
import { Toolbar } from './components/Toolbar';
import { active, close, open, taskOf } from './state/tabs';
import { Home } from './views/Home';
import { Scratchpad } from './views/Scratchpad';
import { Settings } from './views/Settings';
import { NewTask, TaskPage } from './views/Task';

function onKey(e: KeyboardEvent) {
  const ctrl = e.ctrlKey || e.metaKey;
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
        </Switch>
      </main>
      <Palette />
    </div>
  );
}
