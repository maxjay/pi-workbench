// The changed files, on the left: how many you have read, what to leave
// out, then the tree. A folder shuts with its arrow, and shut it says how
// many files are in it. A folder that holds only one folder is on the same
// row as it. Long dotted folder names mostly start the same way: that
// start is grey and what tells them apart is white. Only the tree scrolls.

import { createMemo, createSignal, For, Show } from 'solid-js';
import type { ChangedFile } from '../../protocol/ChangedFile';
import { filter, isViewed, setFilter, talksOf } from '../../state/review';
import { excluder, type Pull } from './parts';

interface Folder {
  name: string;
  folders: Map<string, Folder>;
  files: ChangedFile[];
}

/** A piece of a folder's name. The grey pieces are what it shares with the others. */
interface Piece {
  text: string;
  dim: boolean;
}

type Row =
  | { kind: 'folder'; path: string; depth: number; pieces: Piece[]; shut: boolean; count: number }
  | { kind: 'file'; file: ChangedFile; depth: number };

const filesUnder = (folder: Folder): number =>
  folder.files.length + [...folder.folders.values()].reduce((sum, inner) => sum + filesUnder(inner), 0);

export function Tree(props: { p: Pull; current: string | undefined; onPick: (path: string) => void }) {
  const p = () => props.p;
  const files = () => p().review.files;
  const leftOut = createMemo(() => excluder(filter.exclude));
  const kept = createMemo(() => files().filter((file) => !leftOut()(file.path)));
  const listed = createMemo(() => kept().filter((file) => !(filter.unread && isViewed(p().key, file))));
  const read = () => kept().filter((file) => isViewed(p().key, file)).length;
  const [shut, setShut] = createSignal<Record<string, boolean>>({});

  /** The start that every long dotted folder name shares, with its last dot. */
  const shared = createMemo(() => {
    const dotted = new Set<string>();
    for (const file of listed()) {
      for (const part of file.path.split('/').slice(0, -1)) if (part.split('.').length > 2) dotted.add(part);
    }
    const names = [...dotted];
    if (names.length < 2) return '';
    let start = names[0].split('.');
    for (const name of names) {
      const bits = name.split('.');
      let n = 0;
      while (n < start.length && n < bits.length && start[n] === bits[n]) n += 1;
      start = start.slice(0, n);
    }
    // Every folder keeps at least its last word.
    if (names.some((name) => name.split('.').length <= start.length)) start = start.slice(0, -1);
    return start.length > 1 ? `${start.join('.')}.` : '';
  });

  const rows = createMemo((): Row[] => {
    const top: Folder = { name: '', folders: new Map(), files: [] };
    for (const file of listed()) {
      let node = top;
      for (const name of file.path.split('/').slice(0, -1)) {
        let next = node.folders.get(name);
        if (!next) {
          next = { name, folders: new Map(), files: [] };
          node.folders.set(name, next);
        }
        node = next;
      }
      node.files.push(file);
    }
    const out: Row[] = [];
    const pieces = (chain: string[]): Piece[] =>
      chain.flatMap((name, at) => {
        const last = at === chain.length - 1;
        const end = last ? '' : '/';
        const start = shared();
        if (start && name.startsWith(start)) {
          return [
            // Each word of the start can wrap on its own.
            ...start
              .split('.')
              .filter(Boolean)
              .map((bit) => ({ text: `${bit}.`, dim: true })),
            { text: name.slice(start.length) + end, dim: false },
          ];
        }
        return [{ text: name + end, dim: !last }];
      });
    const walk = (node: Folder, depth: number, above: string[]) => {
      for (const first of node.folders.values()) {
        let folder = first;
        const chain = [folder.name];
        while (folder.files.length === 0 && folder.folders.size === 1) {
          folder = folder.folders.values().next().value as Folder;
          chain.push(folder.name);
        }
        const path = [...above, ...chain].join('/');
        const closed = Boolean(shut()[path]);
        out.push({
          kind: 'folder',
          path,
          depth,
          pieces: pieces(chain),
          shut: closed,
          count: filesUnder(folder),
        });
        if (!closed) walk(folder, depth + 1, [...above, ...chain]);
      }
      for (const file of node.files) out.push({ kind: 'file', file, depth });
    };
    walk(top, 0, []);
    return out;
  });

  /** A square for each thing said on a file that still wants you: a thread that is open, and what only you see. */
  const marks = (path: string) => [
    ...p()
      .review.threads.filter((thread) => thread.place?.file === path && thread.unresolved)
      .map(() => 'people'),
    ...talksOf(p().key)
      .filter(
        (talk) =>
          talk.thread === undefined &&
          talk.place?.file === path &&
          !talk.resolved &&
          (talk.said.length > 0 || talk.draft),
      )
      .map(() => 'agent'),
  ];
  const Rails = (r: { depth: number }) => (
    <For each={Array.from({ length: r.depth })}>{() => <span class="ft-rail" aria-hidden="true" />}</For>
  );

  return (
    <div class="files">
      <div class="files-head">
        <span>Files</span>
        <span class="files-read">
          {read()} of {kept().length} read
        </span>
      </div>
      <div class="files-rule" aria-hidden="true" />
      <div class="files-filter">
        <input
          type="text"
          aria-label="Files to leave out"
          placeholder="leave out: tests/**"
          title="Patterns split by commas, such as tests/**, *.md"
          value={filter.exclude}
          onInput={(e) => setFilter({ exclude: e.currentTarget.value })}
        />
        <button
          type="button"
          class="quiet files-unread"
          title="Only the files you have not read"
          aria-pressed={filter.unread}
          onClick={() => setFilter({ unread: !filter.unread })}
        >
          unread
        </button>
      </div>
      <span class="files-out">
        {files().length - kept().length > 0 ? `${files().length - kept().length} left out` : ''}
      </span>
      <div class="files-tree">
        <For each={rows()} fallback={<span class="files-none">No files.</span>}>
          {(row) =>
            row.kind === 'folder' ? (
              <button
                type="button"
                class="row ft-row"
                title={row.path}
                aria-expanded={!row.shut}
                onClick={() => setShut({ ...shut(), [row.path]: !row.shut })}
              >
                <Rails depth={row.depth} />
                <span class="ft-arrow" aria-hidden="true">
                  <svg
                    width="9"
                    height="9"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="3"
                  >
                    <title>{row.shut ? 'Shut' : 'Open'}</title>
                    <path d="M6 9l6 6 6-6" />
                  </svg>
                </span>
                <span class="ft-folder">
                  <For each={row.pieces}>
                    {(piece) => <span data-dim={piece.dim || undefined}>{piece.text}</span>}
                  </For>
                </span>
                <span class="ft-count">{row.shut ? row.count : ''}</span>
              </button>
            ) : (
              <button
                type="button"
                class="row ft-row ft-file"
                title={row.file.path}
                aria-current={props.current === row.file.path ? 'true' : undefined}
                data-read={isViewed(p().key, row.file) || undefined}
                onClick={() => props.onPick(row.file.path)}
              >
                <Rails depth={row.depth} />
                <span class="ft-name">{row.file.path.slice(row.file.path.lastIndexOf('/') + 1)}</span>
                <span class="ft-marks" aria-hidden="true">
                  <For each={marks(row.file.path)}>{(tone) => <span class="sq" data-tone={tone} />}</For>
                </span>
                <Show when={row.file.loaded}>
                  <span class="counts">
                    <span data-state="ok">+{row.file.added}</span>
                    <Show when={row.file.removed > 0}>
                      {' '}
                      <span data-state="failed">-{row.file.removed}</span>
                    </Show>
                  </span>
                </Show>
                <span class="ft-tick">{isViewed(p().key, row.file) ? '✓' : ''}</span>
              </button>
            )
          }
        </For>
      </div>
    </div>
  );
}
