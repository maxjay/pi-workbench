// wbd's log file, as the Log tab shows it. The file is read only while that
// tab is shown: once a second, and only what was written since the last time.
// wbd also says what runs now, which the file says only once something ended.

import { createSignal } from 'solid-js';
import { ask, onEvent } from '../daemon';
import type { LogRunning } from '../protocol/LogRunning';

/** One thing in the file: its line, and the lines that carry it on. */
export interface Entry {
  text: string;
  /** Where its outcome starts in `text`, when it failed or still runs: what comes after is coloured. */
  outcome?: { at: number; state: 'failed' | 'running' };
}

/** How many are kept here. The file has the rest. */
const KEPT = 5000;

const [entries, setEntries] = createSignal<Entry[]>([]);
const [running, setRunning] = createSignal<LogRunning[]>([]);
/** Where the file is. */
const [file, setFile] = createSignal('');
/** When the read that is not answered yet was sent. */
const [asked, setAsked] = createSignal<number>();
let after = 0;

export { asked, entries, file, running };

// A wbd that started again is asked for the file's end again: it holds the run before too.
onEvent((event) => {
  if (event.event !== 'ready') return;
  after = 0;
  setEntries([]);
});

function entryOf(text: string): Entry {
  const first = text.split('\n', 1)[0];
  const failed = first.indexOf('  failed after ');
  if (failed >= 0) return { text, outcome: { at: failed, state: 'failed' } };
  const still = first.search(/ {2}still running after \d+ s$/);
  return still >= 0 ? { text, outcome: { at: still, state: 'running' } } : { text };
}

async function read() {
  setAsked(Date.now());
  const answer = await ask({ method: 'log.read', params: { after } }, 'log');
  setAsked();
  after = answer.end;
  setRunning(answer.running);
  setFile(answer.file);
  if (!answer.text) return;
  const next = [...entries()];
  for (const line of answer.text.replace(/\n$/, '').split('\n')) {
    const last = next[next.length - 1];
    // A line that starts with spaces carries on the one before it.
    if (line.startsWith(' ') && last) next[next.length - 1] = entryOf(`${last.text}\n${line}`);
    else next.push(entryOf(line));
  }
  setEntries(next.slice(-KEPT));
}

/** Reads the file once a second until the function it gives back is called. */
export function watchLog() {
  let on = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const turn = async () => {
    // wbd being down is said in the top row.
    await read().catch(() => setAsked());
    if (on) timer = setTimeout(turn, 1000);
  };
  void turn();
  return () => {
    on = false;
    clearTimeout(timer);
  };
}
