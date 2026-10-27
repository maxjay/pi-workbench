// What Ctrl K can do: go to a tab or a part of Settings, or run an action.
// The palette only filters and runs these; they're defined here.

import { reconnect } from '../agent';
import { openFolderDialog } from '../components/OpenFolder';
import { openPalette } from '../components/Palette';
import { activePage, addressOf, navigate, onWeb, openPage, suggestions } from './browser';
import { codespaces, openFolder } from './codespaces';
import { scratchpads, showScratchpad } from './sessions';
import { reload, sections, setSection } from './settings';
import { open, tabs, title } from './tabs';
import { tasks } from './tasks';

export interface Command {
  group: 'Address' | 'History' | 'Tabs' | 'Codespaces' | 'Scratchpads' | 'Tasks' | 'Settings' | 'Actions';
  title: string;
  detail?: string;
  run: () => void;
}

export function commands(): Command[] {
  return [
    ...tabs().map(
      (tab): Command => ({ group: 'Tabs', title: title(tab), detail: 'open tab', run: () => open(tab) }),
    ),
    ...codespaces().map(
      (view): Command => ({
        group: 'Codespaces',
        title: view.space.name,
        detail: view.space.path,
        run: () => void openFolder(view.space.path).then((v) => open(`space:${v.space.id}`)),
      }),
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
    ...tasks().map(
      (view): Command => ({
        group: 'Tasks',
        title: view.task.name,
        detail: 'task',
        run: () => open(`task:${view.task.id}`),
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
    {
      group: 'Actions',
      title: 'New web tab',
      detail: 'a page, with Pi beside it',
      run: () => void openPage('about:blank').then(() => openPalette()),
    },
    { group: 'Actions', title: 'Open folder', detail: 'as a codespace', run: openFolderDialog },
    {
      group: 'Actions',
      title: 'New task',
      detail: 'a prompt Pi runs on a schedule',
      run: () => open('newtask'),
    },
    { group: 'Actions', title: 'Reload Pi’s files', detail: 'after editing them in a terminal', run: reload },
    { group: 'Actions', title: 'Restart wb-agent', run: () => void reconnect() },
  ];
}

/** Going to what's typed, as an address or a search: in this web tab, or a new one. */
function address(query: string): Command[] {
  const typed = query.trim();
  if (!typed) return [];
  const url = addressOf(typed);
  const searching = url.startsWith('https://duckduckgo.com/?q=');
  // Away from a web tab, only what looks like an address.
  if (searching && !onWeb()) return [];
  const page = activePage();
  return [
    {
      group: 'Address',
      title: searching ? `Search for “${typed}”` : `Go to ${typed}`,
      detail: page ? 'in this tab' : 'in a new web tab',
      run: () => void (page ? navigate(page.id, typed) : openPage(typed)),
    },
  ];
}

/** Pages from your history that match, as wb-agent last found them. */
function history(): Command[] {
  return suggestions().map((visit) => ({
    group: 'History',
    title: visit.title || visit.url,
    detail: visit.url.replace(/^https?:\/\//, ''),
    run: () => {
      const page = activePage();
      void (page ? navigate(page.id, visit.url) : openPage(visit.url));
    },
  }));
}

/** The commands whose title or detail contain every word typed. On a web
 * tab, going to what's typed and your history come first. */
export function search(query: string): Command[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const found = commands().filter((command) => {
    const text = `${command.title} ${command.detail ?? ''} ${command.group}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
  // The page's own address is what the bar opens on: nothing to search yet.
  const atPage = onWeb() && activePage()?.url.replace(/^https?:\/\//, '') === query.trim();
  return [...(atPage ? [] : address(query)), ...(onWeb() ? history() : []), ...found];
}
