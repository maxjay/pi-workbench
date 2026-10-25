// What `@` and `/` list as you type, as in Pi's terminal: a file under the
// folder Pi works in, for a word starting with @, or one of Pi's commands,
// for a message starting with /. Picking one puts it in place of what was
// typed for it.

import type { PiCommand } from '../../protocol/PiCommand';

export interface Choice {
  main: string;
  detail: string;
  tag: string;
  /** What replaces the typed word. */
  insert: string;
}

export interface Picker {
  kind: 'files' | 'commands';
  /** Where the typed word starts, in the text before the caret. */
  start: number;
  query: string;
  choices: Choice[];
}

const SHOWN = 8;

/** The list for the text before the caret, if it's asking for one. */
export function picker(before: string, files: string[], commands: PiCommand[]): Picker | undefined {
  const slash = /^\/(\S*)$/.exec(before);
  if (slash) {
    const query = slash[1].toLowerCase();
    const choices = commands
      .filter((c) => c.name.toLowerCase().includes(query))
      .sort(
        (a, b) =>
          Number(b.name.toLowerCase().startsWith(query)) - Number(a.name.toLowerCase().startsWith(query)),
      )
      .slice(0, SHOWN)
      .map((c) => ({
        main: `/${c.name}`,
        detail: c.description ?? '',
        tag: c.source,
        insert: `/${c.name} `,
      }));
    return { kind: 'commands', start: 0, query: slash[1], choices };
  }
  const at = /(^|\s)@(\S*)$/.exec(before);
  if (at) {
    const query = at[2].toLowerCase();
    const rank = (path: string) => {
      const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase();
      if (name.startsWith(query)) return 0;
      if (name.includes(query)) return 1;
      return path.toLowerCase().includes(query) ? 2 : 3;
    };
    const choices = files
      .map((path) => ({ path, rank: rank(path) }))
      .filter((f) => f.rank < 3)
      .sort((a, b) => a.rank - b.rank || a.path.length - b.path.length || a.path.localeCompare(b.path))
      .slice(0, SHOWN)
      .map(({ path }) => {
        const cut = path.lastIndexOf('/');
        return {
          main: path.slice(cut + 1),
          detail: cut > 0 ? path.slice(0, cut) : '',
          tag: '',
          insert: `@${path} `,
        };
      });
    return { kind: 'files', start: before.length - at[2].length - 1, query: at[2], choices };
  }
  return undefined;
}
