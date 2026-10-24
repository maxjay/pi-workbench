import { For, Show } from 'solid-js';
import { active, close, focus, open, type TabKind, tabs } from '../state/tabs';
import { Icon, type IconName } from './Icon';

const icons: Record<TabKind, IconName> = { home: 'home', settings: 'settings' };

export function TabStrip() {
  return (
    <div class="tabstrip" role="tablist" aria-label="Open tabs">
      <For each={tabs()}>
        {(tab) => (
          <div class="tab" role="presentation" data-active={active() === tab.kind}>
            <button
              type="button"
              role="tab"
              class="tab-main"
              aria-selected={active() === tab.kind}
              onClick={() => focus(tab.kind)}
            >
              <Icon name={icons[tab.kind]} />
              <span class="tab-title">{tab.title}</span>
            </button>
            <Show when={tab.kind !== 'home'}>
              <button
                type="button"
                class="tab-close"
                aria-label={`Close ${tab.title}`}
                onClick={() => close(tab.kind)}
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
