// The one row on top: the mark, the open tabs, and Settings. It says where
// wbd stands only while wbd is starting or has stopped. What wbd found is
// in Settings.

import { For, Show } from 'solid-js';
import { status, stopReason } from '../daemon';
import { codespaceById } from '../state/codespaces';
import { standing } from '../state/conversations';
import { titles } from '../state/review';
import { active, close, codespaceOf, marks, open, pullOf, type TabId, tabs, ticketOf } from '../state/tabs';
import { tickets } from '../state/tickets';
import { Glyph } from './Glyph';
import { pullHue, ticketHue } from './hues';

const words = () => (status() === 'connecting' ? 'Starting wbd' : (stopReason() ?? 'wbd stopped'));

/** The number of the pull request or the ticket a tab shows. */
const numberOf = (tab: TabId) => pullOf(tab)?.pr ?? ticketOf(tab)?.id;

/** What a tab is called: Home, its codespace's name, or its pull request's or ticket's number and title. */
export function titleOf(tab: TabId) {
  if (tab === 'settings') return 'Settings';
  const numbered = numberOf(tab);
  if (numbered !== undefined) return titles[tab] ? `#${numbered} ${titles[tab]}` : `#${numbered}`;
  const id = codespaceOf(tab);
  return id ? (codespaceById(id)?.codespace.name ?? 'Codespace') : 'Home';
}

/**
 * A tab's own colour: a pull request's is its own, a ticket's is its type's. Its number is written
 * in it, and it is the line under the tab while you are on it. The other tabs keep the app's.
 */
function hueOfTab(tab: TabId) {
  const pull = pullOf(tab);
  if (pull) return pullHue(pull.pr);
  const ticket = ticketOf(tab);
  if (!ticket) return undefined;
  // Until its page has read it, Home's list may already know its type.
  const listed = tickets.latest?.find((row) => row.id === ticket.id);
  return marks[tab]?.hue ?? (listed ? ticketHue(listed.type) : undefined);
}

/** Where a tab's agent stands: waiting on you, or working. */
function dotOf(tab: TabId) {
  const id = codespaceOf(tab);
  const view = id ? codespaceById(id) : undefined;
  return view ? standing(view.codespace.path) : marks[tab]?.standing;
}

export function TopBar() {
  return (
    <header class="top">
      <div class="top-mark">
        <Glyph name="workbench" size={14} />
      </div>
      <nav class="tabs" aria-label="Open tabs">
        <For each={tabs()}>
          {(tab) => (
            <span class="tab" data-current={active() === tab} style={{ '--accent': hueOfTab(tab) }}>
              <button
                type="button"
                class="tab-open"
                aria-current={active() === tab ? 'page' : undefined}
                onClick={() => open(tab)}
              >
                <Show when={codespaceOf(tab)}>
                  <Glyph name={titleOf(tab)} size={10} />
                </Show>
                <Show when={numberOf(tab) !== undefined} fallback={titleOf(tab)}>
                  <span class="tab-id">#{numberOf(tab)}</span>
                  <span class="tab-name">{titles[tab]}</span>
                </Show>
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
      <Show when={status() !== 'ready'}>
        <span class="top-status" data-status={status()}>
          {words()}
        </span>
      </Show>
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
