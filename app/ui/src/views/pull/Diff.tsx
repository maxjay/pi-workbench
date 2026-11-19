// One changed file's diff, in the middle: old and new in one column, or
// side by side. A click picks a line, and shift-click picks every line up
// to it: the chat on the right is then told you are on them. A right-click
// gives Comment and Ask. What is said about lines is under the last of
// them, one line until you open it. Left of the line numbers each thing
// said has a thin bar down its lines, side by side where two overlap, and
// a resolved one is a small square there.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { createAction } from '../../components/action';
import type { DiffLine } from '../../protocol/DiffLine';
import type { Place } from '../../protocol/Place';
import type { Thread } from '../../protocol/Thread';
import {
  addTalk,
  dropTalk,
  isViewed,
  markViewed,
  setSplit,
  split,
  type Talk,
  talksOf,
  threadTalk,
  wholeFile,
} from '../../state/review';
import type { Pull } from './parts';
import { Said } from './Said';

const MARK = { context: ' ', added: '+', removed: '-' } as const;

/** The rows a place covers, first and last. */
interface Span {
  lo: number;
  hi: number;
}

/** Something said about lines of this file. */
interface Card {
  key: string;
  thread?: Thread;
  talk?: Talk;
  place: Place;
  span: Span | undefined;
  resolved: boolean;
  tone: 'people' | 'agent';
}

/** A row of the side by side view: a line of the old file, a line of the new, or both. */
interface Pair {
  left: number | undefined;
  right: number | undefined;
}

export function Diff(props: {
  p: Pull;
  path: string;
  /** A line of the new file to show, when the file was opened at one. */
  at: number | undefined;
}) {
  const p = () => props.p;
  const file = () => p().review.files.find((f) => f.path === props.path);
  // Every line of the file, once you ask for it. Until then: the changes and what is near them.
  const [whole, setWhole] = createSignal<DiffLine[]>();
  const loading = createAction(async () => setWhole(await wholeFile(p().dir, p().pr, props.path)));
  // A push that changes the file makes the whole of it stale.
  createEffect(
    on(
      () => file()?.mark,
      () => setWhole(),
      { defer: true },
    ),
  );
  const lines = () => whole() ?? file()?.lines ?? [];

  /** The row each line number is on, in the old file and in the new. */
  const rows = createMemo(() => {
    const [old, fresh] = [new Map<number, number>(), new Map<number, number>()];
    lines().forEach((line, row) => {
      if (line.old !== null) old.set(line.old, row);
      if (line.new !== null) fresh.set(line.new, row);
    });
    return { old, fresh };
  });
  const spanOf = (place: Place): Span | undefined => {
    if (place.line === null) return undefined;
    const side = place.old ? rows().old : rows().fresh;
    const hi = side.get(place.line);
    if (hi === undefined) return undefined;
    return { lo: side.get(place.from ?? place.line) ?? hi, hi };
  };

  /** The lines you picked, as the first and last row. */
  const [picked, setPicked] = createSignal<{ a: number; b: number }>();
  const pickedSpan = (): Span | undefined => {
    const range = picked();
    return range && { lo: Math.min(range.a, range.b), hi: Math.max(range.a, range.b) };
  };
  /** Where rows are, as Azure DevOps counts them. Old lines only when every one of them was removed. */
  const placeOf = (span: Span): Place | undefined => {
    const covered = lines().slice(span.lo, span.hi + 1);
    const old = covered.every((line) => line.kind === 'removed');
    const numbers = covered.map((line) => (old ? line.old : line.new)).filter((n): n is number => n !== null);
    if (numbers.length === 0) return undefined;
    return { file: props.path, from: numbers[0], line: numbers[numbers.length - 1], old, outdated: false };
  };
  const pick = (row: number, extend: boolean) => {
    const range = picked();
    // A click on the one line that is picked lets go of it.
    if (!extend && range && range.a === row && range.b === row) {
      setPicked();
      if (p().about()?.kind === 'lines') p().setAbout(undefined);
      return;
    }
    const next = extend && range ? { a: range.a, b: row } : { a: row, b: row };
    setPicked(next);
    const place = placeOf({ lo: Math.min(next.a, next.b), hi: Math.max(next.a, next.b) });
    if (place) p().setAbout({ kind: 'lines', place });
  };
  // The chat was told about something else: the lines are let go of.
  createEffect(() => {
    if (p().about()?.kind !== 'lines') setPicked();
  });

  let scroller!: HTMLDivElement;
  // Opened at a line: it is picked and brought into view.
  createEffect(
    on(
      () => [props.at, lines().length] as const,
      ([at]) => {
        const row = at === undefined ? undefined : rows().fresh.get(at);
        if (row === undefined) return;
        setPicked({ a: row, b: row });
        requestAnimationFrame(() =>
          scroller.querySelector(`[data-row="${row}"]`)?.scrollIntoView({ block: 'center' }),
        );
      },
    ),
  );

  /** Everything said about this file's lines: its threads, and what only you and the agent said. */
  const cards = createMemo((): Card[] => {
    const out: Card[] = [];
    for (const thread of p().review.threads) {
      if (thread.place?.file !== props.path) continue;
      out.push({
        key: `t${thread.id}`,
        thread,
        talk: threadTalk(p().key, thread.id),
        place: thread.place,
        span: spanOf(thread.place),
        resolved: !thread.unresolved,
        tone: 'people',
      });
    }
    for (const talk of talksOf(p().key)) {
      if (talk.thread !== undefined || talk.place?.file !== props.path) continue;
      out.push({
        key: `k${talk.id}`,
        talk,
        place: talk.place,
        span: spanOf(talk.place),
        resolved: Boolean(talk.resolved),
        tone: 'agent',
      });
    }
    return out;
  });
  const byKey = createMemo(() => new Map(cards().map((card) => [card.key, card])));
  /** A resolved one is not drawn under its lines until you open it. */
  const drawn = (card: Card) => !card.resolved || p().opened(card.key);
  /** The keys of what is drawn under each row. Keys, so a card stays itself while what is said in it changes. */
  const under = createMemo(() => {
    const at = new Map<number, string[]>();
    for (const card of cards()) {
      if (!card.span || !drawn(card)) continue;
      at.set(card.span.hi, [...(at.get(card.span.hi) ?? []), card.key]);
    }
    return at;
  });
  /** What has no line shown to be under: it is above the code. */
  const loose = createMemo(() =>
    cards()
      .filter((card) => !card.span && (drawn(card) || card.thread))
      .map((card) => card.key),
  );
  /** The resolved ones that end on each row: a square left of the line numbers opens them. */
  const shut = createMemo(() => {
    const at = new Map<number, string>();
    for (const card of cards()) if (card.span && !drawn(card)) at.set(card.span.hi, card.key);
    return at;
  });
  /** Each thing said gets one of three places left of the line numbers, so two that overlap are side by side. */
  const lanes = createMemo(() => {
    const at = new Map<number, (string | undefined)[]>();
    const ends: number[] = [];
    const drawnCards = cards()
      .filter((card) => card.span && drawn(card))
      .sort((a, b) => (a.span as Span).lo - (b.span as Span).lo || (a.span as Span).hi - (b.span as Span).hi);
    for (const card of drawnCards) {
      const span = card.span as Span;
      let lane = ends.findIndex((end) => end < span.lo);
      if (lane < 0) lane = ends.length < 3 ? ends.length : 2;
      ends[lane] = span.hi;
      for (let row = span.lo; row <= span.hi; row++) {
        const here = at.get(row) ?? [];
        here[lane] = card.resolved ? 'faint' : card.tone;
        at.set(row, here);
      }
    }
    return at;
  });

  /** The lines of the card the pointer is on. */
  const [lit, setLit] = createSignal<{ span: Span; tone: string }>();
  const litAt = (row: number) => {
    const now = lit();
    return now && row >= now.span.lo && row <= now.span.hi ? now.tone : undefined;
  };
  const isPicked = (row: number) => {
    const span = pickedSpan();
    return span !== undefined && row >= span.lo && row <= span.hi;
  };

  /** The list a right-click gives: on which row, and how far from the left. */
  const [menu, setMenu] = createSignal<{ row: number; left: number }>();
  createEffect(() => {
    if (!menu()) return;
    const away = (e: Event) => {
      if (!(e.target as Element).closest?.('.dl-menu')) setMenu();
    };
    const key = (e: KeyboardEvent) => e.key === 'Escape' && setMenu();
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', key);
    onCleanup(() => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', key);
    });
  });
  const openMenu = (row: number, e: MouseEvent) => {
    e.preventDefault();
    // A right-click on lines that are picked is about all of them.
    if (!isPicked(row)) pick(row, false);
    const line = e.currentTarget as HTMLElement;
    const box = (line.closest('.dl') ?? line).getBoundingClientRect();
    setMenu({ row, left: Math.max(64, Math.min(e.clientX - box.left, box.width - 170)) });
  };
  /** Starts something said about the picked lines: asked of the agent, or written to post. */
  const begin = (ask: boolean) => {
    const span = pickedSpan();
    const place = span && placeOf(span);
    setMenu();
    if (!place) return;
    const id = addTalk(p().key, { place, said: [], ask });
    p().setOpened(`k${id}`, true);
    setPicked();
    p().setAbout(undefined);
  };
  const close = (card: Card) => {
    p().setOpened(card.key, false);
    // Nothing was said in it: it is not kept.
    const talk = card.talk;
    if (!card.thread && talk && talk.said.length === 0 && !talk.draft) dropTalk(p().key, talk.id);
  };

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
  const Gap = (g: { row: number }) => (
    <Show when={gapBefore(g.row) > 0}>
      <button type="button" class="dl-gap" disabled={loading.busy()} onClick={() => void loading.run()}>
        {gapBefore(g.row)} {gapBefore(g.row) === 1 ? 'line' : 'lines'}
      </button>
    </Show>
  );

  /** Left of the line numbers: the bar of what is picked, then a bar for each thing said on the row. */
  const Gutter = (g: { rows: number[] }) => {
    const lane = (k: number) => g.rows.map((row) => lanes().get(row)?.[k]).find(Boolean);
    const closed = () => g.rows.map((row) => shut().get(row)).find(Boolean);
    return (
      <>
        <span class="dl-bar" data-on={g.rows.some(isPicked) || undefined} />
        <span class="dl-lanes">
          <i data-tone={lane(0)} />
          <i data-tone={lane(1)} />
          <i data-tone={lane(2)} />
          <Show when={closed()}>
            {(key) => (
              <button
                type="button"
                class="dl-shut"
                aria-label="Open what was resolved here"
                title="Resolved"
                onClick={() => p().setOpened(key(), true)}
              >
                <span class="sq" data-tone="faint" />
              </button>
            )}
          </Show>
        </span>
      </>
    );
  };

  /** One line of code. With a side, it is one half of a row of two and has only that side's number. */
  const Code = (c: { row: number; side?: 'old' | 'new' }) => {
    const line = () => lines()[c.row] as DiffLine;
    return (
      // biome-ignore lint/a11y/noStaticElementInteractions: the line's number is the button; the rest of the line is a bigger place to click
      // biome-ignore lint/a11y/useKeyWithClickEvents: the number beside it takes the keys
      <div
        class="dl-code"
        data-kind={line().kind}
        data-side={c.side}
        data-on={isPicked(c.row) || undefined}
        data-lit={litAt(c.row)}
        data-row={c.row}
        onMouseDown={(e) => {
          // Shift-click picks every line up to this one. It does not select their text.
          if (e.shiftKey) e.preventDefault();
        }}
        onClick={(e) => {
          // Dragging over the code selects it, to copy. Only a plain click picks the line.
          if (!e.shiftKey && window.getSelection()?.isCollapsed === false) return;
          pick(c.row, e.shiftKey);
        }}
        onContextMenu={(e) => openMenu(c.row, e)}
      >
        <Show when={c.side !== 'new'}>
          <span class="dl-num dl-old">{line().old ?? ''}</span>
        </Show>
        <Show when={c.side !== 'old'}>
          <button
            type="button"
            class="dl-num"
            aria-label={`Line ${line().new ?? line().old}`}
            onClick={(e) => {
              e.stopPropagation();
              pick(c.row, e.shiftKey);
            }}
          >
            {line().new ?? ''}
          </button>
        </Show>
        <span class="dl-mark">{c.side && line().kind === 'context' ? ' ' : MARK[line().kind]}</span>
        <span class="dl-text">{line().text || ' '}</span>
      </div>
    );
  };

  const Menu = (m: { rows: number[] }) => (
    <Show when={m.rows.includes(menu()?.row ?? -1)}>
      <div class="dl-menu" style={{ left: `${menu()?.left ?? 0}px` }}>
        <button type="button" class="row" onClick={() => begin(false)}>
          Comment
        </button>
        <button type="button" class="row" onClick={() => begin(true)}>
          Ask
        </button>
      </div>
    </Show>
  );

  /** What is said under a row, or above the code. */
  const Cards = (c: { keys: string[]; loose?: boolean }) => (
    <For each={c.keys}>
      {(key) => (
        <Show when={byKey().get(key)}>
          {(card) => (
            <div class="dl-card" data-split={split() || undefined} data-loose={c.loose || undefined}>
              <Said
                p={p()}
                thread={card().thread}
                talk={card().talk}
                place={card().place}
                open={p().opened(key)}
                showPlace={c.loose}
                gone={c.loose && (Boolean(whole()) || card().place.outdated || card().place.line === null)}
                onOpen={() => {
                  p().setOpened(key, true);
                  if (card().thread) p().setAbout({ kind: 'thread', thread: (card().thread as Thread).id });
                }}
                onClose={() => close(card())}
                onThread={(id) => p().setOpened(`t${id}`, true)}
                onLight={(on) => {
                  const span = card().span;
                  setLit(on && span ? { span, tone: card().resolved ? 'faint' : card().tone } : undefined);
                }}
              />
            </div>
          )}
        </Show>
      )}
    </For>
  );

  return (
    <Show
      when={file()}
      fallback={<p class="pr-pad dim">This pull request no longer changes {props.path}.</p>}
    >
      {(f) => (
        <div class="diff">
          <div class="diff-head">
            <span class="diff-path" title={f().path}>
              <Show when={f().oldPath}>{(old) => <span class="dim">{old()} → </span>}</Show>
              <span class="dim diff-dir">{f().path.slice(0, f().path.lastIndexOf('/') + 1)}</span>
              <span class="diff-name">{f().path.slice(f().path.lastIndexOf('/') + 1)}</span>
            </span>
            <span class="counts">
              <span data-state="ok">+{f().added}</span>{' '}
              <Show when={f().removed > 0}>
                <span data-state="failed">-{f().removed}</span>
              </Show>
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
            </Show>
            <button
              type="button"
              class="quiet"
              aria-pressed={isViewed(p().key, f())}
              onClick={() => markViewed(p().key, [f()], !isViewed(p().key, f()))}
            >
              {isViewed(p().key, f()) ? '✓ read' : 'Read'}
            </button>
          </div>

          <div class="diff-body" ref={scroller} onScroll={() => menu() && setMenu()}>
            <Show when={loading.error()}>{(error) => <p class="pr-pad note-failed">{error()}</p>}</Show>
            <Show when={f().binary}>
              <p class="pr-pad dim">Not text. There are no lines to show.</p>
            </Show>
            <Show when={f().large && !whole()}>
              <p class="pr-pad dim">
                This file changes too many lines to send with the rest.{' '}
                <button
                  type="button"
                  class="link"
                  disabled={loading.busy()}
                  onClick={() => void loading.run()}
                >
                  Read it
                </button>
              </p>
            </Show>
            <Cards keys={loose()} loose />
            <Show
              when={split()}
              fallback={
                <For each={lines()}>
                  {(_, row) => (
                    <>
                      <Gap row={row()} />
                      <div class="dl">
                        <Gutter rows={[row()]} />
                        <Code row={row()} />
                        <Menu rows={[row()]} />
                      </div>
                      <Cards keys={under().get(row()) ?? []} />
                    </>
                  )}
                </For>
              }
            >
              <For each={pairs()}>
                {(pair) => {
                  const both = [pair.left, pair.right].filter(
                    (row, at, all): row is number => row !== undefined && all.indexOf(row) === at,
                  );
                  return (
                    <>
                      <Gap row={pair.left ?? pair.right ?? 0} />
                      <div class="dl dl-pair">
                        <Gutter rows={both} />
                        <Show when={pair.left !== undefined} fallback={<div class="dl-blank" />}>
                          <Code row={pair.left as number} side="old" />
                        </Show>
                        <Show when={pair.right !== undefined} fallback={<div class="dl-blank" />}>
                          <Code row={pair.right as number} side="new" />
                        </Show>
                        <Menu rows={both} />
                      </div>
                      <Cards keys={both.flatMap((row) => under().get(row) ?? [])} />
                    </>
                  );
                }}
              </For>
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}
