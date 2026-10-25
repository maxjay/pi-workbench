import { status, stopReason } from '../agent';
import { health } from '../state/health';
import { current } from '../state/sessions';
import { section, sections } from '../state/settings';
import { active, open } from '../state/tabs';
import { Icon } from './Icon';
import { openPalette } from './Palette';

export function Toolbar() {
  const where = () => {
    if (active() === 'settings') return ['Settings', `› ${sections.find((s) => s.id === section())?.title}`];
    if (active() === 'scratch') {
      const title = current()?.state.title;
      return ['Scratchpad', title ? `› ${title}` : ''];
    }
    return ['Home', ''];
  };
  const agent = () => {
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
