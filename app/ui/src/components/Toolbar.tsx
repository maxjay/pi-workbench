import { Show } from 'solid-js';
import { status, stopReason } from '../agent';
import { activePage, chatHidden, goBack, goForward, onWeb, reload, toggleChat } from '../state/browser';
import { spaceById } from '../state/codespaces';
import { health } from '../state/health';
import { repos } from '../state/repo';
import { abort, current } from '../state/sessions';
import { section, sections } from '../state/settings';
import { active, kindOf, open, spaceOf, title } from '../state/tabs';
import { uiOf } from '../views/space/ui';
import { Downloads } from './Downloads';
import { Icon } from './Icon';
import { openPalette } from './Palette';

export function Toolbar() {
  const page = activePage;
  const where = () => {
    const web = page();
    if (onWeb()) {
      const bare = (web?.url ?? '').replace(/^https?:\/\//, '');
      const host = bare.split('/')[0];
      return [host || 'New tab', bare.slice(host.length)];
    }
    if (active() === 'settings')
      return [
        'Settings',
        `› ${sections.find((s) => s.id === section())?.title ?? (section() === 'browser' ? 'Browser' : 'Background')}`,
      ];
    if (active() === 'scratch') {
      const pad = current()?.state.title;
      return ['Scratchpad', pad ? `› ${pad}` : ''];
    }
    if (kindOf(active()) === 'task') return ['Tasks', `› ${title(active())}`];
    const space = spaceOf(active());
    const view = space ? spaceById(space) : undefined;
    if (space && view) {
      const branch = repos[view.space.path]?.git?.summary.branch ?? view.git?.branch;
      const file = uiOf(space).file?.split('/').pop();
      return [
        view.space.name,
        [branch, file]
          .filter(Boolean)
          .map((part) => `› ${part}`)
          .join(' '),
      ];
    }
    return ['Home', ''];
  };
  const agent = () => {
    const space = spaceOf(active());
    const view = space ? spaceById(space) : undefined;
    if (view && status() === 'ready')
      return view.running.length > 0 ? `${view.running.join(', ')} running` : 'Nothing running';
    if (status() === 'connecting') return 'Starting wb-agent…';
    if (status() === 'down') return 'wb-agent stopped';
    const h = health();
    if (!h) return 'wb-agent ready';
    return [h.distro ?? 'Linux', h.pi ? `pi ${h.pi}` : 'pi not found'].join(' · ');
  };
  return (
    <div class="toolbar">
      <span class="logo">π</span>
      {/* On a web tab the bar is its address, with the browser's controls around it. */}
      <Show when={onWeb() && page()}>
        {(web) => (
          <div class="web-nav">
            <button
              type="button"
              class="icon-button"
              aria-label="Back"
              disabled={!web().canBack}
              onClick={() => void goBack(web().id)}
            >
              <Icon name="back" size={15} />
            </button>
            <button
              type="button"
              class="icon-button"
              aria-label="Forward"
              disabled={!web().canForward}
              onClick={() => void goForward(web().id)}
            >
              <Icon name="forward" size={15} />
            </button>
            <button
              type="button"
              class="icon-button"
              aria-label="Reload"
              onClick={() => void reload(web().id)}
            >
              <Icon name="reload" size={14} />
            </button>
          </div>
        )}
      </Show>
      <button
        type="button"
        class="bar"
        title={onWeb() ? 'Go to an address, or anything else (Ctrl K)' : 'Go to anything (Ctrl K)'}
        onClick={() => openPalette(onWeb() ? (page()?.url ?? '').replace(/^https?:\/\//, '') : '')}
      >
        <Icon name="search" size={14} />
        <span class="bar-main">{where()[0]}</span>
        <span class="bar-rest">{where()[1]}</span>
        <span class="bar-key">Ctrl K</span>
      </button>
      <Show when={onWeb() && page()?.piDriving}>
        <span class="web-driving">
          <span class="web-driving-dot" aria-hidden="true" />
          Pi is driving
          <Show when={page()?.session}>
            {(session) => (
              <button type="button" class="button web-take-back" onClick={() => void abort(session())}>
                Take back
              </button>
            )}
          </Show>
        </span>
      </Show>
      <Show
        when={onWeb()}
        fallback={
          <span class="toolbar-status" data-status={status()} title={stopReason()}>
            {agent()}
          </span>
        }
      >
        <Downloads />
        <button
          type="button"
          class="icon-button"
          aria-label={chatHidden() ? 'Show Pi beside the page' : 'Hide Pi: just the browser'}
          title={chatHidden() ? 'Show Pi beside the page' : 'Hide Pi: just the browser'}
          aria-pressed={!chatHidden()}
          data-on={!chatHidden()}
          onClick={toggleChat}
        >
          <Icon name="panel" size={16} />
        </button>
      </Show>
      <button
        type="button"
        class="icon-button"
        aria-label="Settings"
        aria-pressed={active() === 'settings'}
        onClick={() => open('settings')}
      >
        <Icon name="settings" size={16} />
      </button>
    </div>
  );
}
