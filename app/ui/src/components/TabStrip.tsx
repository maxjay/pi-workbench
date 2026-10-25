import { For, Show } from 'solid-js';
import {
  active,
  close,
  focus,
  kindOf,
  open,
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
};

/** A task tab's dot: amber while Pi runs it, blue while it waits on you. */
function dot(id: TabId) {
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
