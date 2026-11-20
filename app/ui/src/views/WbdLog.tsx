// The Log tab: what wbd did, the oldest on top. Each call from the page, each
// thing asked of dova, each git command and each program it ran, with how
// long it took or has run so far, and why it failed. It is where to look when
// something waits and does not say why.

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { copy } from '../components/code';
import type { LogEntry } from '../protocol/LogEntry';
import { asked, clock, entries, watchLog } from '../state/log';

type Only = 'all' | 'running' | 'failed';

/** Who did it, as the row says it. A call is something the page asked. */
const who: Record<LogEntry['kind'], string> = {
  call: 'page',
  dova: 'dova',
  git: 'git',
  run: 'run',
  wbd: 'wbd',
};

const time = (at: number) => new Date(at).toLocaleTimeString('en-GB');

/** How long, in the largest two units that say something. */
function span(ms: number, whole = false) {
  if (ms < 1000 && !whole) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${whole ? Math.floor(s) : s.toFixed(1)} s`;
  return `${Math.floor(s / 60)} min ${Math.floor(s % 60)} s`;
}

export function LogPage() {
  const [words, setWords] = createSignal('');
  const [only, setOnly] = createSignal<Only>('all');
  const [tick, setTick] = createSignal(Date.now());
  const [said, setSaid] = createSignal('Copy');
  let list!: HTMLDivElement;
  // The newest stays in view unless you scrolled up to read.
  let following = true;

  onMount(() => {
    const stop = watchLog();
    const timer = setInterval(() => setTick(Date.now()), 1000);
    onCleanup(() => {
      stop();
      clearInterval(timer);
    });
  });

  const count = (which: Only) =>
    entries().filter((entry) => (which === 'running' ? entry.running : entry.failed !== null)).length;
  const shown = createMemo(() => {
    const find = words().trim().toLowerCase();
    return entries().filter((entry) => {
      if (only() === 'running' && !entry.running) return false;
      if (only() === 'failed' && entry.failed === null) return false;
      return !find || line(entry).toLowerCase().includes(find);
    });
  });
  createEffect(() => {
    shown();
    if (following) queueMicrotask(() => list.scrollTo(0, list.scrollHeight));
  });

  /** wbd's clock now: its last answer, and the time since. */
  const now = () => clock().wbd + (tick() - clock().here);
  const took = (entry: LogEntry) => {
    if (entry.running) return `running ${span(Math.max(0, now() - entry.at), true)}`;
    return entry.took === null ? '' : span(entry.took);
  };
  /** One entry as a line of text, for the filter and for Copy. */
  const line = (entry: LogEntry) =>
    [
      time(entry.at),
      who[entry.kind],
      entry.what,
      entry.place,
      took(entry),
      entry.failed && `FAILED: ${entry.failed}`,
    ]
      .filter(Boolean)
      .join('  ');
  /** How long wbd has not answered the read of this log, in seconds. */
  const silent = () => Math.floor((tick() - (asked() ?? tick())) / 1000);

  return (
    <div class="log">
      <div class="log-head">
        <input
          class="field log-find"
          placeholder="Filter"
          aria-label="Filter the log"
          value={words()}
          onInput={(e) => setWords(e.currentTarget.value)}
        />
        <For each={['all', 'running', 'failed'] as Only[]}>
          {(each) => (
            <button type="button" class="quiet" aria-pressed={only() === each} onClick={() => setOnly(each)}>
              {each}
              <Show when={each !== 'all' && count(each) > 0}> {count(each)}</Show>
            </button>
          )}
        </For>
        <span class="top-gap" />
        <button
          type="button"
          class="quiet"
          onClick={async () => {
            setSaid((await copy(shown().map(line).join('\n'))) ? 'Copied' : 'Could not copy');
            setTimeout(() => setSaid('Copy'), 1500);
          }}
        >
          {said()}
        </button>
      </div>
      <Show when={silent() >= 3}>
        <p class="note-failed log-silent">
          wbd has not answered for {silent()} s. It is busy with something that holds it up, or it stopped.
        </p>
      </Show>
      <div
        class="log-list"
        ref={list}
        onScroll={() => {
          following = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
        }}
      >
        <For
          each={shown()}
          fallback={<p class="dim log-none">{entries().length ? 'None match.' : 'Nothing yet.'}</p>}
        >
          {(entry) => (
            <div
              class="log-row"
              data-state={entry.running ? 'running' : entry.failed !== null ? 'failed' : undefined}
            >
              <span class="dim">{time(entry.at)}</span>
              <span class="dim">{who[entry.kind]}</span>
              <span class="log-what">
                {entry.what}
                <Show when={entry.place}>
                  <span class="dim"> {entry.place}</span>
                </Show>
                <Show when={entry.failed}>
                  <span class="log-why">{entry.failed}</span>
                </Show>
              </span>
              <span class="log-took">{took(entry)}</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
