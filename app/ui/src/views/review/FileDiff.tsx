// One changed file's diff, in the middle: old and new in one column, or
// side by side. A line that has comments carries a marker at its left,
// which opens and closes them: a thread still open starts open, resolved
// ones start closed. Click a line to comment on it or to ask the agent
// about it; shift-click another line to cover every line between. What
// the agent answers shows under those lines, for you alone.

import { createEffect, createMemo, createSignal, For, on, Show } from 'solid-js';
import { createAction } from '../../components/action';
import type { DiffLine } from '../../protocol/DiffLine';
import type { Place } from '../../protocol/Place';
import type { Thread } from '../../protocol/Thread';
import {
  comment,
  hideResolved,
  isViewed,
  markViewed,
  type Note,
  notesOf,
  wholeFile,
} from '../../state/review';
import { Read } from './Files';
import { placeName, type Reviewing } from './parts';
import { NoteBox, ThreadBox } from './Threads';

const MARK = { context: ' ', added: '+', removed: '-' } as const;

/** The key a line is found by: its number, on the side the number is counted on. */
const keyOf = (old: boolean, line: number) => `${old ? 'old' : 'new'}:${line}`;

/** Old and new side by side, in every file, until you switch it back. */
const [split, setSplitNow] = createSignal(localStorage.getItem('review-split') === 'yes');

function setSplit(yes: boolean) {
  localStorage.setItem('review-split', yes ? 'yes' : 'no');
  setSplitNow(yes);
}

/** A row of the side by side view: a line of the old file, a line of the new, or both. */
interface Pair {
  left: number | undefined;
  right: number | undefined;
}

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
  const isPicked = (row: number) => {
    const range = span();
    return range !== undefined && row >= range.lo && row <= range.hi;
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
      { place: at, question: text().trim() },
    );
    setPicked();
    setText('');
  };

  /** This file's threads. */
  const here = () => props.r.review.threads.filter((thread) => thread.place?.file === props.path);
  // Threads and notes are found by the line they end on, so a line draws its own.
  const threads = createMemo(() => {
    const at = new Map<string, Thread[]>();
    for (const thread of here()) {
      const where = thread.place;
      if (!where || where.line === null) continue;
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
  const under = <T,>(found: Map<string, T[]>, line: DiffLine | undefined) => [
    ...((line && line.new !== null && found.get(keyOf(false, line.new))) || []),
    ...((line && line.old !== null && found.get(keyOf(true, line.old))) || []),
  ];
  const draftFor = (thread: Thread) => notesOf(props.r.pull).find((note) => note.thread === thread.id);
  /** Threads on this file that no line shown draws: on the whole file, on lines gone, or on lines not shown. */
  const loose = createMemo(() => {
    const drawn = new Set(lines().flatMap((line) => under(threads(), line)));
    // With no line to carry a marker, a resolved one follows the overview's switch.
    return here().filter((thread) => !drawn.has(thread) && (thread.unresolved || !hideResolved()));
  });

  /** What a line has under it: a thread still open, the agent's notes, or only resolved threads. */
  const flag = (row: number) => {
    const line = lines()[row];
    const found = under(threads(), line);
    if (found.some((thread) => thread.unresolved)) return 'open';
    if (under(notes(), line).length > 0) return 'note';
    return found.length > 0 ? 'resolved' : undefined;
  };
  // The lines whose comments you opened or closed yourself. The rest follow what is under them.
  const [byHand, setByHand] = createSignal<Record<string, boolean>>({});
  const rowKey = (row: number) => `${lines()[row]?.old ?? ''}:${lines()[row]?.new ?? ''}`;
  const isOpen = (row: number) => byHand()[rowKey(row)] ?? flag(row) !== 'resolved';
  const toggle = (row: number) => setByHand({ ...byHand(), [rowKey(row)]: !isOpen(row) });
  /** A line's threads that are drawn: the open ones while the line is open, and the resolved ones only once you open the line yourself. */
  const shownThreads = (row: number) =>
    isOpen(row)
      ? under(threads(), lines()[row]).filter((thread) => thread.unresolved || byHand()[rowKey(row)] === true)
      : [];
  /** What the agent wrote about lines of this file that are not shown. */
  const looseNotes = createMemo(() => {
    const drawn = new Set(lines().flatMap((line) => under(notes(), line)));
    return notesOf(props.r.pull).filter((note) => note.place?.file === props.path && !drawn.has(note));
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

  /** The lines as rows of two: a run of removed lines beside the run of added lines that took its place. */
  const pairs = createMemo(() => {
    const [all, out] = [lines(), [] as Pair[]];
    let row = 0;
    while (row < all.length) {
      if (all[row].kind === 'context') {
        out.push({ left: row, right: row });
        row += 1;
        continue;
      }
      const [removed, added] = [[] as number[], [] as number[]];
      while (all[row]?.kind === 'removed') removed.push(row++);
      while (all[row]?.kind === 'added') added.push(row++);
      for (let k = 0; k < Math.max(removed.length, added.length); k++) {
        out.push({ left: removed[k], right: added[k] });
      }
    }
    return out;
  });

  /** A gap in the file before a row, which reads the rest of the file when clicked. */
  const Gap = (p: { row: number }) => (
    <Show when={gapBefore(p.row) > 0}>
      <button type="button" class="diff-gap" onClick={() => void loading.run()}>
        {gapBefore(p.row)} {gapBefore(p.row) === 1 ? 'line' : 'lines'}
      </button>
    </Show>
  );

  /** One line. With a side, only that side's number: it is one half of a row of two. */
  const Line = (p: { row: number; side?: 'old' | 'new' }) => {
    const line = () => lines()[p.row];
    return (
      <Show when={line()}>
        {(l) => (
          // biome-ignore lint/a11y/noStaticElementInteractions: the line's number is the button; the rest of the line is a bigger place to click
          // biome-ignore lint/a11y/useKeyWithClickEvents: the number beside it takes the keys
          <div
            class="diff-line"
            data-kind={l().kind}
            data-side={p.side}
            data-picked={isPicked(p.row) || undefined}
            onClick={(e) => {
              // Dragging over the code selects it, to copy. Only a plain click picks the line.
              if (window.getSelection()?.isCollapsed === false) return;
              pick(p.row, e.shiftKey);
            }}
          >
            <Show
              when={flag(p.row) && (p.side === 'old') === (p.side !== undefined && l().kind === 'removed')}
              fallback={<span />}
            >
              <button
                type="button"
                class="diff-flag"
                data-what={flag(p.row)}
                aria-expanded={isOpen(p.row)}
                aria-label={
                  isOpen(p.row) ? 'Close the comments on this line' : 'Open the comments on this line'
                }
                title={
                  flag(p.row) === 'resolved' ? 'Resolved' : flag(p.row) === 'open' ? 'Open' : 'From the agent'
                }
                onClick={(e) => {
                  e.stopPropagation();
                  toggle(p.row);
                }}
              >
                <span class="flag" data-what={flag(p.row)} />
              </button>
            </Show>
            <Show when={p.side !== 'new'}>
              <span class="diff-num">{l().old ?? ''}</span>
            </Show>
            <Show when={p.side !== 'old'}>
              <button
                type="button"
                class="diff-num"
                aria-label={`Comment on line ${l().new ?? l().old}`}
                onClick={(e) => {
                  e.stopPropagation();
                  pick(p.row, e.shiftKey);
                }}
              >
                {l().new ?? ''}
              </button>
            </Show>
            <span class="diff-mark">{MARK[l().kind]}</span>
            <span class="diff-text">{l().text || ' '}</span>
          </div>
        )}
      </Show>
    );
  };

  /** What is drawn under a line: the box you are writing in, its threads, the agent's notes. */
  const Under = (p: { row: number }) => (
    <>
      <Show when={span()?.hi === p.row}>
        <div class="diff-under">
          <form
            class="diff-write"
            onSubmit={(e) => {
              e.preventDefault();
              void posting.run();
            }}
          >
            <span class="diff-write-where" title="Shift-click another line to cover more">
              <Show when={place()} fallback="These lines have no number to comment on">
                {(at) => placeName(at())}
              </Show>
            </span>
            <textarea
              class="diff-write-text"
              rows={2}
              placeholder="Comment, or ask the agent"
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
                Ask
              </button>
              <button type="button" class="quiet" onClick={() => setPicked()}>
                Cancel
              </button>
            </div>
          </form>
        </div>
      </Show>
      <For each={shownThreads(p.row)}>
        {(thread) => (
          <div class="diff-under">
            <ThreadBox r={props.r} thread={thread} draft={draftFor(thread)} />
          </div>
        )}
      </For>
      <For each={isOpen(p.row) ? under(notes(), lines()[p.row]) : []}>
        {(note) => (
          <div class="diff-under">
            <NoteBox
              note={note}
              pull={props.r.pull}
              label="agent"
              onPost={(words) => comment(props.r.dir, props.r.pr, words, note.place ?? null)}
            />
          </div>
        )}
      </For>
    </>
  );

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
            <Show when={f().change !== 'edited'}>
              <span class="dim">{f().change}</span>
            </Show>
            <span class="top-gap" />
            <Show when={!f().binary}>
              <button
                type="button"
                class="quiet"
                title="Old and new side by side"
                aria-pressed={split()}
                onClick={() => setSplit(!split())}
              >
                split
              </button>
              <button
                type="button"
                class="quiet"
                title="Every line of the file, not only the changes"
                aria-pressed={Boolean(whole())}
                disabled={loading.busy()}
                onClick={() => (whole() ? setWhole() : void loading.run())}
              >
                full
              </button>
            </Show>
            <Read
              on={isViewed(props.r.pull, f())}
              name={f().path}
              onToggle={() => markViewed(props.r.pull, [f()], !isViewed(props.r.pull, f()))}
            />
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
            <For each={looseNotes()}>
              {(note) => (
                <div class="diff-under">
                  <NoteBox
                    note={note}
                    pull={props.r.pull}
                    label={`agent · ${note.place ? placeName(note.place) : props.path}`}
                    onPost={(words) => comment(props.r.dir, props.r.pr, words, note.place ?? null)}
                  />
                </div>
              )}
            </For>

            <Show
              when={split()}
              fallback={
                <For each={lines()}>
                  {(_, row) => (
                    <>
                      <Gap row={row()} />
                      <Line row={row()} />
                      <Under row={row()} />
                    </>
                  )}
                </For>
              }
            >
              <For each={pairs()}>
                {(pair) => (
                  <>
                    <Gap row={pair.left ?? pair.right ?? 0} />
                    <div class="diff-pair">
                      <Show when={pair.left !== undefined} fallback={<div class="diff-blank" />}>
                        <Line row={pair.left as number} side="old" />
                      </Show>
                      <Show when={pair.right !== undefined} fallback={<div class="diff-blank" />}>
                        <Line row={pair.right as number} side="new" />
                      </Show>
                    </div>
                    <Show when={pair.left !== undefined}>
                      <Under row={pair.left as number} />
                    </Show>
                    <Show when={pair.right !== undefined && pair.right !== pair.left}>
                      <Under row={pair.right as number} />
                    </Show>
                  </>
                )}
              </For>
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}
