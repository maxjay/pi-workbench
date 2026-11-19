// The changed files, on the left: grouped by folder, each with what it
// adds and removes and how many threads on it are still open. A box marks
// a file, or a whole folder, as viewed. One switch hides what you have
// viewed, and one field leaves files out by pattern.

import { createMemo, createSignal, For, Show } from 'solid-js';
import type { ChangedFile } from '../../protocol/ChangedFile';
import { isViewed, markViewed, notesOf } from '../../state/review';
import { byFolder, excluder, type Reviewing, split } from './parts';

export function Files(props: {
  r: Reviewing;
  /** The file shown in the middle, if one is. */
  current: string | undefined;
  onPick: (path: string) => void;
}) {
  const [exclude, setExclude] = createSignal(localStorage.getItem('review-exclude') ?? '');
  const [hideViewed, setHideViewed] = createSignal(false);
  const [closed, setClosed] = createSignal<string[]>([]);

  const files = () => props.r.review.files;
  const viewed = (file: ChangedFile) => isViewed(props.r.pull, file);
  const shown = createMemo(() => {
    const out = excluder(exclude());
    return files().filter((file) => !out(file.path) && !(hideViewed() && viewed(file)));
  });
  /** How many threads on each file still want an answer. */
  const open = createMemo(() => {
    const counts = new Map<string, number>();
    for (const thread of props.r.review.threads) {
      if (thread.unresolved && thread.place)
        counts.set(thread.place.file, (counts.get(thread.place.file) ?? 0) + 1);
    }
    return counts;
  });
  /** How many comments the agent has drafted on each file, waiting for you. */
  const drafted = createMemo(() => {
    const counts = new Map<string, number>();
    for (const note of notesOf(props.r.pull)) {
      if (note.place && note.answer) counts.set(note.place.file, (counts.get(note.place.file) ?? 0) + 1);
    }
    return counts;
  });
  const hidden = () => files().length - shown().length;

  const toggleFolder = (dir: string) =>
    setClosed(closed().includes(dir) ? closed().filter((d) => d !== dir) : [...closed(), dir]);
  const viewFolder = (dir: string, list: ChangedFile[]) => {
    const all = list.every(viewed);
    markViewed(props.r.pull, list, !all);
    // A folder you are done with gets out of the way.
    if (!all && !closed().includes(dir)) toggleFolder(dir);
  };

  return (
    <div class="files">
      <div class="files-head">
        <span>
          Files · {files().filter(viewed).length} of {files().length} viewed
          <Show when={hidden() > 0}> · {hidden()} hidden</Show>
        </span>
        <button type="button" class="quiet" onClick={() => setHideViewed(!hideViewed())}>
          {hideViewed() ? 'show viewed' : 'hide viewed'}
        </button>
      </div>
      <input
        class="files-exclude"
        placeholder="files to exclude: tests/**, *.md"
        aria-label="Files to exclude"
        value={exclude()}
        onInput={(e) => {
          setExclude(e.currentTarget.value);
          localStorage.setItem('review-exclude', e.currentTarget.value);
        }}
      />
      <For each={byFolder(shown())} fallback={<p class="files-none dim">No files match.</p>}>
        {(folder) => (
          <>
            <Show when={folder.dir}>
              <div class="files-row">
                <button
                  type="button"
                  class="files-folder"
                  aria-expanded={!closed().includes(folder.dir)}
                  onClick={() => toggleFolder(folder.dir)}
                >
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    aria-hidden="true"
                  >
                    <path stroke-width="2" d="M6 9l6 6 6-6" />
                  </svg>
                  <span class="tree-name">{folder.dir}</span>
                  <Show when={closed().includes(folder.dir)}>
                    <span>
                      {folder.files.length} {folder.files.length === 1 ? 'file' : 'files'}
                    </span>
                  </Show>
                </button>
                <ViewedBox
                  on={folder.files.every(viewed)}
                  name={folder.dir}
                  onToggle={() => viewFolder(folder.dir, folder.files)}
                />
              </div>
            </Show>
            <Show when={!closed().includes(folder.dir)}>
              <For each={folder.files}>
                {(file) => (
                  <div class="files-row" data-current={props.current === file.path}>
                    <button
                      type="button"
                      class="files-file"
                      classList={{
                        'files-top': !folder.dir,
                        dim: viewed(file) && props.current !== file.path,
                      }}
                      title={file.path}
                      onClick={() => props.onPick(file.path)}
                    >
                      <span class="tree-name">{split(file.path).name}</span>
                      <Show when={drafted().get(file.path)}>
                        {(n) => <span class="files-drafted">{n()} drafted</span>}
                      </Show>
                      <Show when={open().get(file.path)}>
                        {(n) => <span class="files-open">{n()} open</span>}
                      </Show>
                      <span class="files-count">
                        <Show when={file.added > 0}>
                          <span data-state="ok">+{file.added}</span>
                        </Show>
                        <Show when={file.removed > 0}>
                          <span data-state="failed"> -{file.removed}</span>
                        </Show>
                        <Show when={file.added + file.removed === 0}>
                          <span class="dim">{file.binary ? 'binary' : file.change}</span>
                        </Show>
                      </span>
                    </button>
                    <ViewedBox
                      on={viewed(file)}
                      name={file.path}
                      onToggle={() => markViewed(props.r.pull, [file], !viewed(file))}
                    />
                  </div>
                )}
              </For>
            </Show>
          </>
        )}
      </For>
    </div>
  );
}

function ViewedBox(props: { on: boolean; name: string; onToggle: () => void }) {
  return (
    <button
      type="button"
      class="viewed-box"
      aria-pressed={props.on}
      aria-label={`Viewed: ${props.name}`}
      title={props.on ? 'Viewed. Click to unmark' : 'Mark as viewed'}
      onClick={props.onToggle}
    >
      <span />
    </button>
  );
}
