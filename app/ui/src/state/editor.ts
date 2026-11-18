// The files open in editors, by absolute path: what is on disk as last read,
// whether yours differs, and whether the file changed on disk while yours
// had changes you hadn't saved. The text itself lives in the editor. This
// keeps what saving and noticing changes need.

import { createStore } from 'solid-js/store';
import { ask, DaemonError } from '../daemon';
import type { FileContent } from '../protocol/FileContent';

export interface Buffer {
  /** As read, or as written. Only text is edited. */
  content: FileContent;
  disk: string;
  modified: number;
  dirty: boolean;
  /** What is on disk now, when it changed under changes you hadn't saved. */
  changedOnDisk: string | null;
}

const [buffers, setBuffers] = createStore<Record<string, Buffer>>({});

export { buffers, setBuffers };

const textOf = (content: FileContent) => (content.kind === 'text' ? content.text : '');

const read = (path: string) => ask({ method: 'file.read', params: { path } }, 'file');

/** Reads a file into its buffer, the first time it is opened. */
export async function load(path: string) {
  const view = await read(path);
  setBuffers(path, {
    content: view.content,
    disk: textOf(view.content),
    modified: view.modified,
    dirty: false,
    changedOnDisk: null,
  });
  return buffers[path];
}

/**
 * Saves what you typed, unless the file changed on disk since: then the
 * buffer says so, and nothing is written.
 */
export async function save(path: string, text: string) {
  const buffer = buffers[path];
  if (!buffer) return;
  try {
    const call = { method: 'file.write', params: { path, text, expected: buffer.disk } } as const;
    const { modified } = await ask(call, 'saved');
    setBuffers(path, { disk: text, modified, dirty: false, changedOnDisk: null });
  } catch (err) {
    if (!(err instanceof DaemonError) || err.kind !== 'conflict') throw err;
    const view = await read(path);
    setBuffers(path, { changedOnDisk: textOf(view.content), modified: view.modified });
  }
}

/**
 * Checks the open files against the disk. One that changed is read again:
 * the new text, for the editor to show, when yours had nothing unsaved.
 * Otherwise it is kept aside until you choose.
 */
export async function checkDisk(paths: string[]): Promise<Map<string, string>> {
  const fresh = new Map<string, string>();
  const known = paths.filter((path) => buffers[path]?.content.kind === 'text');
  if (known.length === 0) return fresh;
  const { modified } = await ask({ method: 'files.stat', params: { paths: known } }, 'stat');
  for (const [i, path] of known.entries()) {
    const buffer = buffers[path];
    if (!buffer || modified[i] === buffer.modified || modified[i] === 0) continue;
    const view = await read(path);
    const text = textOf(view.content);
    if (text === buffer.disk) {
      setBuffers(path, 'modified', view.modified);
    } else if (buffer.dirty) {
      setBuffers(path, { changedOnDisk: text, modified: view.modified });
    } else {
      setBuffers(path, { disk: text, content: view.content, modified: view.modified });
      fresh.set(path, text);
    }
  }
  return fresh;
}

/** Keeps yours: the next save writes over what changed on disk. */
export function keepMine(path: string) {
  const changed = buffers[path]?.changedOnDisk;
  if (changed !== null && changed !== undefined) setBuffers(path, { disk: changed, changedOnDisk: null });
}

/** Takes what is on disk, dropping what you hadn't saved. Returns it, for the editor. */
export function takeTheirs(path: string) {
  const changed = buffers[path]?.changedOnDisk;
  if (changed === null || changed === undefined) return undefined;
  setBuffers(path, { disk: changed, dirty: false, changedOnDisk: null });
  return changed;
}

export function forgetBuffer(path: string) {
  setBuffers(path, undefined as unknown as Buffer);
}
