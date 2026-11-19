// One changed file's diff, in the middle. A thread is drawn under the line
// it is on. Click a line to comment on it or to ask the agent about it;
// shift-click another line to cover every line between. What the agent
// answers shows under those lines, for you alone.

import { createEffect, createMemo, createSignal, For, on, Show } from 'solid-js';
import { createAction } from '../../components/action';
import type { DiffLine } from '../../protocol/DiffLine';
import type { Place } from '../../protocol/Place';
import type { Thread } from '../../protocol/Thread';
import { comment, isViewed, markViewed, type Note, notesOf, wholeFile } from '../../state/review';
import { placeName, type Reviewing } from './parts';
import { NoteBox, ThreadBox } from './Threads';

const MARK = { context: ' ', added: '+', removed: '-' } as const;

/** The key a line is found by: its number, on the side the number is counted on. */
const keyOf = (old: boolean, line: number) => `${old ? 'old' : 'new'}:${line}`;

export function FileDiff(props: {
  r: Reviewing;
  path: string;
  /** A line of the new file to start writing on, when the file was opened at one. */
  at: number | undefined;
}) {
  const file = () => props.r.review.files.find((f) => f.path === props.path);
  // Every line of the file, once you ask for it. Until then: the changes and what is near them.
  const [whole, setWhole] = createSignal<DiffLine[]>();
  const loading = createAction(async () => setWhole(await wholeFile(props.r.dir, props.r.pr, props.path)));
  // A push that changes the file makes the whole of it stale.
  createEffect(
    on(
      () => file()?.mark,
      () => setWhole(),
      { defer: true },
    ),
  );
  const lines = () => whole() ?? file()?.lines ?? [];

  /** The lines you are writing on, as the first and last row. */
  const [picked, setPicked] = createSignal<{ a: number; b: number }>();
  const [text, setText] = createSignal('');
  const span = () => {
    const range = picked();
    return range && { lo: Math.min(range.a, range.b), hi: Math.max(range.a, range.b) };
  };
  const pick = (row: number, extend: boolean) => {
    const range = picked();
    setPicked(extend && range ? { a: range.a, b: row } : { a: row, b: row });
  };
  // Opened at a line: start writing there.
  createEffect(
    on(
      () => props.at,
      (at) => {
        const row = at === undefined ? -1 : lines().findIndex((line) => line.new === at);
        if (row >= 0) setPicked({ a: row, b: row });
      },
    ),
  );

  /** Where a new thread on the picked rows goes. Old lines only when every picked line was removed. */
  const place = (): Place | undefined => {
    const range = span();
    if (!range) return undefined;
    const rows = lines().slice(range.lo, range.hi + 1);
    const old = rows.every((row) => row.kind === 'removed');
    const numbers = rows.map((row) => (old ? row.old : row.new)).filter((n): n is number => n !== null);
    if (numbers.length === 0) return undefined;
    return { file: props.path, from: numbers[0], line: numbers[numbers.length - 1], old, outdated: false };
  };
  const posting = createAction(async () => {
    const [at, words] = [place(), text().trim()];
    if (!at || !words) return;
    await comment(props.r.dir, props.r.pr, words, at);
    setPicked();
    setText('');
  });
  const askAbout = () => {
    const [at, range] = [place(), span()];
    if (!at || !range) return;
    const code = lines()
      .slice(range.lo, range.hi + 1)
      .map((row) => `${MARK[row.kind]}${row.text}`)
      .join('\n');
    const question = text().trim() || 'Explain these lines, and say if anything is wrong with them.';
    props.r.askAgent(
      `About ${placeName(at)} in this pull request. The lines, as the diff has them:\n\n\`\`\`\n${code}\n\`\`\`\n\n${question}`,
      { place: at, question },
    );
    setPicked();
    setText('');
  };

  // Threads and notes are found by the line they end on, so a line draws its own.
  const threads = createMemo(() => {
    const at = new Map<string, Thread[]>();
    for (const thread of props.r.review.threads) {
      const where = thread.place;
      if (where?.file !== props.path || where.line === null) continue;
      const key = keyOf(where.old, where.line);
      at.set(key, [...(at.get(key) ?? []), thread]);
    }
    return at;
  });
  const notes = createMemo(() => {
    const at = new Map<string, Note[]>();
    for (const note of notesOf(props.r.pull)) {
      const where = note.place;
      if (where?.file !== props.path || where.line === null) continue;
      const key = keyOf(where.old, where.line);
      at.set(key, [...(at.get(key) ?? []), note]);
    }
    return at;
  });
  const under = <T,>(found: Map<string, T[]>, line: DiffLine) => [
    ...((line.new !== null && found.get(keyOf(false, line.new))) || []),
    ...((line.old !== null && found.get(keyOf(true, line.old))) || []),
  ];
  const draftFor = (thread: Thread) => notesOf(props.r.pull).find((note) => note.thread === thread.id);
  /** Threads on this file that no line shown draws: on the whole file, on lines gone, or on lines not shown. */
  const loose = createMemo(() => {
    const drawn = new Set(lines().flatMap((line) => under(threads(), line)));
    return props.r.review.threads.filter((thread) => thread.place?.file === props.path && !drawn.has(thread));
  });

  /** How many lines of the file are left out just before a row. */
  const gapBefore = (row: number) => {
    const [line, before] = [lines()[row], lines()[row - 1]];
    if (!line) return 0;
    if (!before) return (line.new ?? line.old ?? 1) - 1;
    if (line.new !== null && before.new !== null) return line.new - before.new - 1;
    if (line.old !== null && before.old !== null) return line.old - before.old - 1;
    return 0;
  };

  return (
    <Show
      when={file()}
      fallback={<p class="review-pad dim">This pull request no longer changes {props.path}.</p>}
    >
      {(f) => (
        <div class="diff">
          <div class="bar diff-head">
            <span class="tree-name">
              <Show when={f().oldPath}>{(old) => <span class="dim">{old()} → </span>}</Show>
              {f().path}
            </span>
            <span class="files-count">
              <span data-state="ok">+{f().added}</span> <span data-state="failed">-{f().removed}</span>
            </span>
            <span class="dim">{f().change}</span>
            <span class="top-gap" />
            <Show when={!f().binary}>
              <button
                type="button"
                class="quiet"
                disabled={loading.busy()}
                onClick={() => (whole() ? setWhole() : void loading.run())}
              >
                {loading.busy() ? 'Reading the file' : whole() ? 'Only the changes' : 'The whole file'}
              </button>
            </Show>
            <button
              type="button"
              class="quiet"
              onClick={() => markViewed(props.r.pull, [f()], !isViewed(props.r.pull, f()))}
            >
              {isViewed(props.r.pull, f()) ? 'Viewed. Unmark' : 'Mark as viewed'}
            </button>
          </div>

          <div class="diff-body">
            <Show when={loading.error()}>{(error) => <p class="review-pad note-failed">{error()}</p>}</Show>
            <Show when={f().binary}>
              <p class="review-pad dim">Not text. There are no lines to show.</p>
            </Show>
            <Show when={f().large && !whole()}>
              <p class="review-pad dim">
                This file changes too many lines to send with the rest.{' '}
                <button type="button" class="pull-open" onClick={() => void loading.run()}>
                  Read it
                </button>
              </p>
            </Show>
            <For each={loose()}>
              {(thread) => (
                <div class="diff-under">
                  <ThreadBox r={props.r} thread={thread} draft={draftFor(thread)} showPlace />
                </div>
              )}
            </For>

            <For each={lines()}>
              {(line, row) => (
                <>
                  <Show when={gapBefore(row()) > 0}>
                    <button type="button" class="diff-gap" onClick={() => void loading.run()}>
                      {gapBefore(row())} {gapBefore(row()) === 1 ? 'line' : 'lines'} not shown
                    </button>
                  </Show>
                  {/* biome-ignore lint/a11y/noStaticElementInteractions: the line's number is the button; the rest of the line is a bigger place to click */}
                  {/* biome-ignore lint/a11y/useKeyWithClickEvents: the number beside it takes the keys */}
                  <div
                    class="diff-line"
                    data-kind={line.kind}
                    data-picked={
                      (span() && row() >= (span()?.lo ?? 0) && row() <= (span()?.hi ?? -1)) || undefined
                    }
                    onClick={(e) => {
                      // Dragging over the code selects it, to copy. Only a plain click picks the line.
                      if (window.getSelection()?.isCollapsed === false) return;
                      pick(row(), e.shiftKey);
                    }}
                  >
                    <span class="diff-num">{line.old ?? ''}</span>
                    <button
                      type="button"
                      class="diff-num"
                      aria-label={`Comment on line ${line.new ?? line.old}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        pick(row(), e.shiftKey);
                      }}
                    >
                      {line.new ?? ''}
                    </button>
                    <span class="diff-mark">{MARK[line.kind]}</span>
                    <span class="diff-text">{line.text || ' '}</span>
                  </div>

                  <Show when={span()?.hi === row()}>
                    <div class="diff-under">
                      <form
                        class="diff-write"
                        onSubmit={(e) => {
                          e.preventDefault();
                          void posting.run();
                        }}
                      >
                        <span class="diff-write-where">
                          <Show when={place()} fallback="These lines have no number to comment on">
                            {(at) => placeName(at())}
                          </Show>{' '}
                          · shift-click another line to cover more
                        </span>
                        <textarea
                          class="diff-write-text"
                          rows={2}
                          placeholder="Comment, or ask the agent about it"
                          aria-label="Comment or question"
                          value={text()}
                          onInput={(e) => setText(e.currentTarget.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Escape') setPicked();
                            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void posting.run();
                          }}
                          ref={(el) => requestAnimationFrame(() => el.focus())}
                        />
                        <Show when={posting.error()}>
                          {(error) => <span class="diff-write-where note-failed">{error()}</span>}
                        </Show>
                        <div class="diff-write-foot">
                          <button
                            type="submit"
                            class="button button-accent"
                            disabled={posting.busy() || !text().trim() || !place()}
                          >
                            Comment
                          </button>
                          <button type="button" class="button" disabled={!place()} onClick={askAbout}>
                            Ask the agent
                          </button>
                          <button type="button" class="quiet" onClick={() => setPicked()}>
                            Cancel
                          </button>
                        </div>
                      </form>
                    </div>
                  </Show>

                  <For each={under(threads(), line)}>
                    {(thread) => (
                      <div class="diff-under">
                        <ThreadBox r={props.r} thread={thread} draft={draftFor(thread)} />
                      </div>
                    )}
                  </For>
                  <For each={under(notes(), line)}>
                    {(note) => (
                      <div class="diff-under">
                        <NoteBox
                          note={note}
                          pull={props.r.pull}
                          label={`You asked the agent${note.place ? ` · ${placeName(note.place)}` : ''} · only you see it`}
                          post="Post as a comment"
                          onPost={(words) => comment(props.r.dir, props.r.pr, words, note.place ?? null)}
                        />
                      </div>
                    )}
                  </For>
                </>
              )}
            </For>
          </div>
        </div>
      )}
    </Show>
  );
}
