// What Ctrl K can do: go to a tab or a part of Settings, or run an action.
// The palette only filters and runs these; they're defined here.

import { reconnect } from '../agent';
import { scratchpads, showScratchpad } from './sessions';
import { reload, sections, setSection } from './settings';
import { open, tabs } from './tabs';

export interface Command {
  group: 'Tabs' | 'Scratchpads' | 'Settings' | 'Actions';
  title: string;
  detail?: string;
  run: () => void;
}

export function commands(): Command[] {
  return [
    ...tabs().map(
      (tab): Command => ({ group: 'Tabs', title: tab.title, detail: 'open tab', run: () => open(tab.kind) }),
    ),
    ...scratchpads().map(
      (pad): Command => ({
        group: 'Scratchpads',
        title: pad.title,
        detail: pad.busy ? 'Pi is working' : undefined,
        run: () => {
          open('scratch');
          void showScratchpad(pad);
        },
      }),
    ),
    ...sections.map(
      (section): Command => ({
        group: 'Settings',
        title: section.title,
        detail: section.file,
        run: () => {
          setSection(section.id);
          open('settings');
        },
      }),
    ),
    { group: 'Actions', title: 'New scratchpad', detail: 'Ctrl N', run: () => open('scratch') },
    { group: 'Actions', title: 'Reload Pi’s files', detail: 'after editing them in a terminal', run: reload },
    { group: 'Actions', title: 'Restart wb-agent', run: () => void reconnect() },
  ];
}

/** The commands whose title or detail contain every word typed. */
export function search(query: string): Command[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return commands().filter((command) => {
    const text = `${command.title} ${command.detail ?? ''} ${command.group}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
}
