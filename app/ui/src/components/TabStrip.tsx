import { For, Show } from 'solid-js';
import { spaceById } from '../state/codespaces';
import { live } from '../state/sessions';
import {
  active,
  close,
  focus,
  kindOf,
  open,
  spaceOf,
  type TabId,
  type TabKind,
  tabs,
  taskOf,
  title,
} from '../state/tabs';
import { standing, taskById } from '../state/tasks';
import { Icon, type IconName } from './Icon';

const icons: Record<TabKind, IconName> = {
  home: 'home',
  scratch: 'pencil',
  settings: 'settings',
  task: 'clock',
  space: 'folder',
};

/**
 * A task tab's dot: amber while Pi runs it, blue while it waits on you. A
 * codespace's: the same, for any of its sessions.
 */
function dot(id: TabId) {
  const space = spaceOf(id);
  if (space) {
    const path = spaceById(space)?.space.path;
    if (!path) return undefined;
    const here = Object.values(live).filter(
      (s) => s.state.cwd === path || s.state.cwd.startsWith(`${path}/`),
    );
    if (here.some((s) => s.state.ask)) return 'asking';
    return here.some((s) => s.state.busy) ? 'running' : undefined;
  }
  const task = taskOf(id);
  const view = task ? taskById(task) : undefined;
  const now = view ? standing(view) : undefined;
  return now === 'running' || now === 'asking' ? now : undefined;
}

export function TabStrip() {
  return (
    <div class="tabstrip" role="tablist" aria-label="Open tabs">
      <For each={tabs()}>
        {(tab) => (
          <div class="tab" role="presentation" data-active={active() === tab}>
            <button
              type="button"
              role="tab"
              class="tab-main"
              aria-selected={active() === tab}
              onClick={() => focus(tab)}
            >
              <Icon name={icons[kindOf(tab)]} />
              <span class="tab-title">{title(tab)}</span>
              <Show when={dot(tab)}>
                {(now) => (
                  <span
                    class="tab-dot"
                    data-state={now()}
                    role="img"
                    aria-label={now() === 'asking' ? 'Waiting for your answer' : 'Pi is working'}
                  />
                )}
              </Show>
            </button>
            <Show when={tab !== 'home'}>
              <button
                type="button"
                class="tab-close"
                aria-label={`Close ${title(tab)}`}
                onClick={() => close(tab)}
              >
                <Icon name="close" size={11} />
              </button>
            </Show>
          </div>
        )}
      </For>
      <button type="button" class="icon-button" aria-label="New tab" onClick={() => open('home')}>
        <Icon name="plus" size={14} />
      </button>
    </div>
  );
}
