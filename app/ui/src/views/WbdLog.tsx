// The Log tab: the end of wbd's log file, as it is written, and under it what
// runs now. The file has a line for each call from the page, each thing asked
// of dova and each command wbd ran, with how long it took or why it failed.
// It is where to look when something waits and does not say why.

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { copy } from '../components/code';
import { asked, type Entry, entries, file, running, watchLog } from '../state/log';

/** A line starts with its day. The tab leaves it out: the time is enough to read by. */
const DAY = /^\d{4}-\d\d-\d\d /;
/** How wide the time is, in characters. */
const TIME = 12;

/** How long something has run, in whole seconds. */
function span(ms: number) {
  const s = Math.floor(ms / 1000);
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
}

export function LogPage() {
  const [words, setWords] = createSignal('');
  const [failed, setFailed] = createSignal(false);
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

  const failures = () => entries().filter((entry) => entry.outcome?.state === 'failed').length;
  const shown = createMemo(() => {
    const find = words().trim().toLowerCase();
    return entries().filter((entry) => {
      if (failed() && entry.outcome?.state !== 'failed') return false;
      return !find || entry.text.toLowerCase().includes(find);
    });
  });
  createEffect(() => {
    shown();
    running();
    if (following) queueMicrotask(() => list.scrollTo(0, list.scrollHeight));
  });

  /** An entry in three parts: its time, what it was, and how it went if that is to be coloured. */
  const parts = (entry: Entry) => {
    const day = DAY.test(entry.text) ? 11 : 0;
    const at = entry.outcome?.at ?? entry.text.length;
    return {
      time: entry.text.slice(day, day + TIME),
      what: entry.text.slice(day + TIME, at),
      outcome: entry.text.slice(at),
    };
  };
  /** How long wbd has not answered the read of its log, in seconds. */
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
        <button type="button" class="quiet" aria-pressed={!failed()} onClick={() => setFailed(false)}>
          all
        </button>
        <button type="button" class="quiet" aria-pressed={failed()} onClick={() => setFailed(true)}>
          failed
          <Show when={failures() > 0}> {failures()}</Show>
        </button>
        <span class="top-gap" />
        <span class="dim log-file">{file()}</span>
        <button
          type="button"
          class="quiet"
          onClick={async () => {
            const text = shown().map((entry) => entry.text);
            setSaid((await copy(text.join('\n'))) ? 'Copied' : 'Could not copy');
            setTimeout(() => setSaid('Copy'), 1500);
          }}
        >
          {said()}
        </button>
      </div>
      <Show when={silent() >= 3}>
        <p class="note-failed log-note">
          wbd has not answered for {silent()} s. It is busy with something that holds it up, or it stopped.
          The file still shows what it wrote.
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
          fallback={<p class="dim log-note">{entries().length ? 'None match.' : 'Nothing yet.'}</p>}
        >
          {(entry) => (
            <div class="log-row" data-state={entry.outcome?.state}>
              <span class="dim">{parts(entry).time}</span>
              {parts(entry).what}
              <span class="log-outcome">{parts(entry).outcome}</span>
            </div>
          )}
        </For>
        {/* What has not ended is not in the file yet, or only as still running. wbd says it apart. */}
        <For each={running()}>
          {(one) => (
            <div class="log-row" data-state="running">
              <span class="dim">{'running now'.padEnd(TIME)}</span>
              {`  ${one.who.padEnd(4)}  ${one.what}${one.place ? `  ${one.place}` : ''}`}
              <span class="log-outcome">{`  ${span(one.ms)}`}</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
