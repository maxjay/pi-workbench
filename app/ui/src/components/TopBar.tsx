// The one row on top: the mark, the open tabs, Ctrl K, and on the right
// where wbd runs and what it found.

import { For, Show } from 'solid-js';
import { status, stopReason } from '../daemon';
import { health } from '../state/health';
import { active, close, open, type TabId, tabs, workspaceOf } from '../state/tabs';
import { workspaceById } from '../state/workspaces';
import { Glyph } from './Glyph';

function words() {
  if (status() === 'connecting') return 'Starting wbd';
  if (status() === 'down') return stopReason() ?? 'wbd stopped';
  const h = health();
  if (!h) return '';
  const found = [h.pi && 'pi', h.claude && 'claude', h.dova && `dova ${h.dova}`].filter(Boolean);
  return [h.distro ?? 'Linux', ...found].join(' · ');
}

/** What a tab is called: Home, or its workspace's name. */
export function titleOf(tab: TabId) {
  const id = workspaceOf(tab);
  return id ? (workspaceById(id)?.workspace.name ?? 'Workspace') : 'Home';
}

export function TopBar(props: { onGo: () => void }) {
  return (
    <header class="top">
      <div class="top-mark">
        <Glyph name="workbench" size={14} />
      </div>
      <nav class="tabs" aria-label="Open tabs">
        <For each={tabs()}>
          {(tab) => (
            <span class="tab" data-current={active() === tab}>
              <button
                type="button"
                class="tab-open"
                aria-current={active() === tab ? 'page' : undefined}
                onClick={() => open(tab)}
              >
                <Show when={workspaceOf(tab)}>
                  <Glyph name={titleOf(tab)} size={10} />
                </Show>
                {titleOf(tab)}
              </button>
              <Show when={tab !== 'home'}>
                <button
                  type="button"
                  class="strip-close"
                  aria-label={`Close ${titleOf(tab)}`}
                  onClick={() => close(tab)}
                >
                  ×
                </button>
              </Show>
            </span>
          )}
        </For>
      </nav>
      <span class="top-gap" />
      <button type="button" class="go" onClick={props.onGo}>
        <span>Go to anything</span>
        <span>Ctrl K</span>
      </button>
      <span class="top-status" data-status={status()}>
        {words()}
      </span>
    </header>
  );
}
