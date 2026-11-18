// The one row on top: the mark, the open tabs, and on the right where wbd
// runs and which agents it found.

import { status, stopReason } from '../daemon';
import { health } from '../state/health';
import { Glyph } from './Glyph';

function words() {
  if (status() === 'connecting') return 'Starting wbd';
  if (status() === 'down') return stopReason() ?? 'wbd stopped';
  const h = health();
  if (!h) return '';
  const found = [h.pi && 'pi', h.claude && 'claude', h.dova && `dova ${h.dova}`].filter(Boolean);
  return [h.distro ?? 'Linux', ...found].join(' · ');
}

export function TopBar() {
  return (
    <header class="top">
      <div class="top-mark">
        <Glyph name="workbench" size={14} />
      </div>
      <nav class="tabs" aria-label="Open tabs">
        <button type="button" class="tab" aria-current="page">
          Home
        </button>
      </nav>
      <span class="top-gap" />
      <span class="top-status" data-status={status()}>
        {words()}
      </span>
    </header>
  );
}
