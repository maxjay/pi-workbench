// Small things the pull request tab's parts share: what a part needs to
// know about the review it is in, and a few words and patterns.

import type { ChangedFile } from '../../protocol/ChangedFile';
import type { Place } from '../../protocol/Place';
import type { Review } from '../../protocol/Review';
import type { Note } from '../../state/review';

/** The review a part of the tab is in. */
export interface Reviewing {
  /** A folder cloned from the pull request's project: where dova is asked from. */
  dir: string;
  pr: number;
  /** Names this pull request in the window's own storage. */
  pull: string;
  review: Review;
  /**
   * Asks the agent in this pull request's session. The answer shows as a
   * note where `about` says, for you alone.
   */
  askAgent: (prompt: string, about: Omit<Note, 'id' | 'answer' | 'failed'>) => void;
  /** Says something in this pull request's session, and resolves with what the agent ends its turn on. */
  tell: (prompt: string) => Promise<string>;
  /** Shows a changed file, at a line of the new file if one is given. */
  showFile: (path: string, line?: number) => void;
}

/** `src/auth.ts:42`, or `src/auth.ts:42-44`. */
export function placeName(place: Place) {
  if (place.line === null) return place.file;
  const lines = place.from !== null && place.from !== place.line ? `${place.from}-${place.line}` : place.line;
  return `${place.file}:${lines}${place.old ? ' (old)' : ''}`;
}

/** A vote in words, and the state it reads as. */
export function voteWords(vote: number): { words: string; state: 'ok' | 'working' | 'failed' | 'none' } {
  if (vote >= 10) return { words: 'approved', state: 'ok' };
  if (vote > 0) return { words: 'approved with suggestions', state: 'ok' };
  if (vote <= -10) return { words: 'rejected', state: 'failed' };
  if (vote < 0) return { words: 'waiting for the author', state: 'working' };
  return { words: 'no vote', state: 'none' };
}

/** When something was said: the time if it was today, the day if it wasn't. */
export function when(iso: string | null | undefined) {
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
 * "files to exclude", read the way VS Code reads it: patterns split by
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

/** The folder a file is in, and its name. */
export function split(path: string) {
  const cut = path.lastIndexOf('/');
  return { dir: path.slice(0, Math.max(cut, 0)), name: path.slice(cut + 1) };
}

/** Changed files grouped by their folder, the folders by name. */
export function byFolder(files: ChangedFile[]) {
  const folders = new Map<string, ChangedFile[]>();
  for (const file of files) {
    const { dir } = split(file.path);
    folders.set(dir, [...(folders.get(dir) ?? []), file]);
  }
  return [...folders].sort(([a], [b]) => a.localeCompare(b)).map(([dir, list]) => ({ dir, files: list }));
}
