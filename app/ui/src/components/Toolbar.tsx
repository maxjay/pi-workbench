import { status, stopReason } from '../agent';
import { spaceById } from '../state/codespaces';
import { health } from '../state/health';
import { repos } from '../state/repo';
import { current } from '../state/sessions';
import { section, sections } from '../state/settings';
import { active, kindOf, open, spaceOf, title } from '../state/tabs';
import { uiOf } from '../views/space/ui';
import { Icon } from './Icon';
import { openPalette } from './Palette';

export function Toolbar() {
  const where = () => {
    if (active() === 'settings')
      return ['Settings', `› ${sections.find((s) => s.id === section())?.title ?? 'Background'}`];
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
      <button type="button" class="bar" title="Go to anything (Ctrl K)" onClick={() => openPalette()}>
        <Icon name="search" size={14} />
        <span class="bar-main">{where()[0]}</span>
        <span class="bar-rest">{where()[1]}</span>
        <span class="bar-key">Ctrl K</span>
      </button>
      <span class="toolbar-status" data-status={status()} title={stopReason()}>
        {agent()}
      </span>
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
