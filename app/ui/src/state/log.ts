// What wbd did, as the Log tab shows it. It is read only while that tab is
// shown: once a second, and only what is new or changed since the last time.

import { createSignal } from 'solid-js';
import { ask, onEvent } from '../daemon';
import type { LogEntry } from '../protocol/LogEntry';

/** How many are kept here. wbd keeps as many. */
const KEPT = 2000;

const [entries, setEntries] = createSignal<LogEntry[]>([]);
/** wbd's clock when it last answered, and this page's clock at that moment. */
const [clock, setClock] = createSignal({ wbd: 0, here: 0 });
/** When the read that is not answered yet was sent, by this page's clock. */
const [asked, setAsked] = createSignal<number>();
let after = 0;

export { asked, clock, entries };

// A wbd that started again starts its log again.
onEvent((event) => {
  if (event.event !== 'ready') return;
  after = 0;
  setEntries([]);
});

async function read() {
  setAsked(Date.now());
  const answer = await ask({ method: 'log.read', params: { after } }, 'log');
  setAsked();
  setClock({ wbd: answer.now, here: Date.now() });
  after = answer.rev;
  if (answer.entries.length === 0) return;
  const byId = new Map(entries().map((entry) => [entry.id, entry]));
  for (const entry of answer.entries) byId.set(entry.id, entry);
  setEntries([...byId.values()].sort((a, b) => a.id - b.id).slice(-KEPT));
}

/** Reads the log once a second until the function it gives back is called. */
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
