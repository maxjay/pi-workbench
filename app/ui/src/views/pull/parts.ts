// Small things the pull request tab's parts share: what a part needs to
// know about the pull request it is in, and a few words and patterns.

import type { DiffLine } from '../../protocol/DiffLine';
import type { Place } from '../../protocol/Place';
import type { Review } from '../../protocol/Review';
import type { Host } from './talker';

/** What the agent is told you are on. Nothing means the whole pull request. */
export type About =
  | { kind: 'thread'; thread: number }
  | { kind: 'lines'; place: Place }
  | { kind: 'run'; run: number; name: string; context: string };

/** The pull request a part of the tab is in. */
export interface Pull extends Host {
  /** A folder cloned from the pull request's project: where dova is asked from. */
  dir: string;
  pr: number;
  review: Review;
  /** What the chat on the right is told you are on. */
  about: () => About | undefined;
  setAbout: (about: About | undefined) => void;
  /** Puts lines in front of the chat on the right, and the cursor in it. */
  toChat: (place: Place) => void;
  /** Whether a thread or a talk is open, by its key: `t` and the thread's id, `k` and the talk's. */
  opened: (key: string) => boolean;
  setOpened: (key: string, open: boolean) => void;
  /** Shows a changed file, at a line of the new file if one is given. */
  showFile: (path: string, line?: number) => void;
  /** Opens a ticket in a tab of its own. */
  openTicket: (id: number) => void;
  /** Asks Azure DevOps for the pull request again, and waits for it. */
  reload: () => Promise<void>;
}

/** `auth.ts:42`, or `auth.ts:42-44`: the file's name, not its whole path. */
export function placeName(place: Place) {
  const name = place.file.slice(place.file.lastIndexOf('/') + 1);
  if (place.line === null) return name;
  const lines = place.from !== null && place.from !== place.line ? `${place.from}-${place.line}` : place.line;
  return `${name}:${lines}${place.old ? ' (old)' : ''}`;
}

export const samePlace = (a: Place, b: Place) =>
  a.file === b.file && a.from === b.from && a.line === b.line && a.old === b.old;

/** The lines of a file's diff that a place is on. */
export function linesAt(lines: DiffLine[], place: Place) {
  const first = place.from ?? place.line;
  if (first === null || place.line === null) return [];
  const last = place.line;
  return lines.filter((line) => {
    const number = place.old ? line.old : line.new;
    return number !== null && number >= first && number <= last;
  });
}

/** When something was said: the time if it was today, the day if it wasn't. */
export function when(iso: string | number | null | undefined) {
  if (!iso) return '';
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return '';
  const today = at.toDateString() === new Date().toDateString();
  return today
    ? at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : at.toLocaleDateString([], { day: 'numeric', month: 'short' });
}

/** How long something took, from when it started to when it finished. */
export function took(from: string | null | undefined, to: string | null | undefined) {
  if (!from || !to) return '';
  const seconds = Math.round((new Date(to).getTime() - new Date(from).getTime()) / 1000);
  if (!(seconds >= 0)) return '';
  return seconds < 60
    ? `${seconds}s`
    : `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`;
}

/**
 * Files to leave out, read the way VS Code reads it: patterns split by
 * commas, `*` within a name, `**` across folders. A pattern with no slash
 * matches a file or a folder of that name anywhere.
 */
export function excluder(patterns: string): (path: string) => boolean {
  const tests = patterns
    .split(',')
    .map((pattern) =>
      pattern
        .trim()
        .replace(/^\.?\//, '')
        .replace(/\/$/, ''),
    )
    .filter(Boolean)
    .map((pattern) => {
      const source = pattern
        .split(/(\*\*\/|\*\*|\*|\?)/)
        .map((part) => {
          if (part === '**/') return '(?:.*/)?';
          if (part === '**') return '.*';
          if (part === '*') return '[^/]*';
          if (part === '?') return '[^/]';
          return part.replace(/[.+^${}()|[\]\\]/g, '\\$&');
        })
        .join('');
      const whole = new RegExp(`^${source}(?:/.*)?$`);
      if (pattern.includes('/')) return (path: string) => whole.test(path);
      const name = new RegExp(`^${source}$`);
      return (path: string) => path.split('/').some((part) => name.test(part));
    });
  return (path) => tests.some((test) => test(path));
}

/** A colour for a kind of thing: every one has the same lightness and strength, only the hue differs. */
export const hue = (degrees: number) => `oklch(0.8 0.15 ${degrees})`;

/** A pull request's own hue, from its name: one of twelve, the same every time. */
export function hueOf(name: string) {
  let sum = 0;
  for (const letter of name) sum = (sum * 31 + letter.charCodeAt(0)) % 9973;
  // Not the hues that already mean something here: red, orange, yellow, green and the blue of threads.
  const free = [330, 300, 280, 210, 190, 170];
  return free[sum % free.length];
}

/** The hue of a colour written as six hex digits, on the wheel the app's colours are on. */
function hexHue(hex: string) {
  const digits = hex.replace('#', '').slice(-6);
  if (!/^[0-9a-f]{6}$/i.test(digits)) return undefined;
  const [r, g, b] = [0, 2, 4]
    .map((at) => Number.parseInt(digits.slice(at, at + 2), 16) / 255)
    .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const yellow = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return Math.round(((Math.atan2(yellow, a) * 180) / Math.PI + 360) % 360);
}

/**
 * The colour of a kind of ticket: the hue Azure DevOps gives the type when
 * it is known, and else the hue it gives the types every project has.
 */
export function ticketHue(type: string | null | undefined, azure?: string | null) {
  const known = azure ? hexHue(azure) : undefined;
  if (known !== undefined) return hue(known);
  const kind = (type ?? '').toLowerCase();
  if (kind.includes('bug') || kind.includes('issue') || kind.includes('impediment')) return hue(25);
  if (kind.includes('epic')) return hue(45);
  if (kind.includes('feature')) return hue(300);
  if (kind.includes('task')) return hue(95);
  return hue(230);
}

/** A vote, as the word on its button and the state it reads as. */
export function voteWord(vote: number): { word: string; tone: 'ok' | 'working' | 'failed' | 'none' } {
  if (vote >= 10) return { word: 'Approved', tone: 'ok' };
  if (vote > 0) return { word: 'Suggestions', tone: 'ok' };
  if (vote <= -10) return { word: 'Rejected', tone: 'failed' };
  if (vote < 0) return { word: 'Waiting', tone: 'working' };
  return { word: 'Approve', tone: 'none' };
}
