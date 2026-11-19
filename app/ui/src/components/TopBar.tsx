// The one row on top: the mark, the open tabs, Ctrl K, and on the right
// where wbd runs and what it found.

import { For, Show } from 'solid-js';
import { status, stopReason } from '../daemon';
import { standing } from '../state/conversations';
import { health } from '../state/health';
import { titles } from '../state/review';
import { active, close, open, pullOf, type TabId, tabs, workspaceOf } from '../state/tabs';
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

/** What a tab is called: Home, its workspace's name, or its pull request's number and title. */
export function titleOf(tab: TabId) {
  if (tab === 'settings') return 'Settings';
  const pull = pullOf(tab);
  if (pull) return titles[tab] ? `#${pull.pr} ${titles[tab]}` : `#${pull.pr}`;
  const id = workspaceOf(tab);
  return id ? (workspaceById(id)?.workspace.name ?? 'Workspace') : 'Home';
}

/** Where a workspace tab's conversations stand: waiting on you, or working. */
function dotOf(tab: TabId) {
  const id = workspaceOf(tab);
  const view = id ? workspaceById(id) : undefined;
  return view ? standing(view.workspace.path) : undefined;
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
                <Show when={dotOf(tab)}>
                  {(state) => (
                    <span
                      class="dot"
                      data-state={state()}
                      role="img"
                      aria-label={state() === 'asking' ? 'Waiting for your answer' : 'The agent is working'}
                    />
                  )}
                </Show>
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
      <button
        type="button"
        class="top-settings"
        aria-label="Settings"
        aria-pressed={active() === 'settings'}
        onClick={() => open('settings')}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" aria-hidden="true">
          <title>Settings</title>
          <circle cx="12" cy="12" r="3" stroke-width="1.8" />
          <path
            stroke-width="1.8"
            d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"
          />
        </svg>
      </button>
    </header>
  );
}
