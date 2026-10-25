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

export interface Segment {
  text: string;
  mention: boolean;
}

/** Whether `mention` stands as a word of its own at `at` in `text`. */
const standsAlone = (text: string, at: number, mention: string) =>
  (at === 0 || /\s/.test(text[at - 1])) &&
  (at + mention.length === text.length || /\s/.test(text[at + mention.length]));

/** The text in pieces, with what was picked from a list marked, for drawing it highlighted. */
export function segments(text: string, mentions: string[]): Segment[] {
  const found: { at: number; length: number }[] = [];
  for (const mention of mentions) {
    for (let at = text.indexOf(mention); at >= 0; at = text.indexOf(mention, at + 1)) {
      if (mention.startsWith('/') && at !== 0) continue;
      if (
        standsAlone(text, at, mention) &&
        !found.some((f) => at < f.at + f.length && f.at < at + mention.length)
      )
        found.push({ at, length: mention.length });
    }
  }
  found.sort((a, b) => a.at - b.at);
  const pieces: Segment[] = [];
  let from = 0;
  for (const f of found) {
    if (f.at > from) pieces.push({ text: text.slice(from, f.at), mention: false });
    pieces.push({ text: text.slice(f.at, f.at + f.length), mention: true });
    from = f.at + f.length;
  }
  if (from < text.length) pieces.push({ text: text.slice(from), mention: false });
  return pieces;
}

/**
 * What deleting at the caret takes when it's up against a picked mention:
 * the whole of it, so a path never has to be backspaced letter by letter.
 * Undefined when it isn't.
 */
export function deleteMention(
  text: string,
  caret: number,
  backwards: boolean,
  mentions: string[],
): { text: string; caret: number } | undefined {
  for (const mention of mentions) {
    const at = backwards ? caret - mention.length : caret;
    if (at < 0 || text.slice(at, at + mention.length) !== mention || !standsAlone(text, at, mention))
      continue;
    return { text: text.slice(0, at) + text.slice(at + mention.length), caret: at };
  }
  return undefined;
}
