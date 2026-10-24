// The first tab: ask Pi something, and see scratchpads, codespaces, tasks
// and pull requests at a glance. Each list fills in as its stage lands.

import { createSignal, For, Show } from 'solid-js';
import { status, stopReason } from '../agent';
import { createAction } from '../components/action';
import { Empty, ErrorNote } from '../components/Note';
import { health } from '../state/health';
import { scratchpads, showScratchpad, startScratchpad } from '../state/sessions';
import { open } from '../state/tabs';

export function Home() {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    await startScratchpad(text().trim());
    setText('');
    open('scratch');
  });
  const show = createAction(async (pad: Parameters<typeof showScratchpad>[0]) => {
    await showScratchpad(pad);
    open('scratch');
  });
  return (
    <div class="page home">
      <section class="ask">
        <h1>What are we working on?</h1>
        <textarea
          class="ask-box"
          rows={3}
          placeholder="Ask Pi anything. It starts a scratchpad."
          value={text()}
          disabled={start.busy() || status() !== 'ready'}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && text().trim()) {
              e.preventDefault();
              void start.run();
            }
          }}
        />
        <ErrorNote error={start.error()} />
      </section>

      <section class="list">
        <h2>Scratchpads</h2>
        <ErrorNote error={show.error()} />
        <Show when={scratchpads().length > 0} fallback={<Empty>Nothing yet. Ask Pi something above.</Empty>}>
          <ul class="rows">
            <For each={scratchpads().slice(0, 5)}>
              {(pad) => (
                <li>
                  <button type="button" class="row row-button" onClick={() => void show.run(pad)}>
                    <span class="pad-dot" data-state={pad.busy ? 'busy' : pad.id ? 'live' : 'saved'} />
                    <span class="row-title">{pad.title}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      <section class="list">
        <h2>Codespaces</h2>
        <Empty>No codespaces yet. Opening a folder in WSL as a codespace comes in stage 3.</Empty>
      </section>

      <section class="list">
        <h2>Tasks</h2>
        <Empty>No tasks yet. Prompts Pi runs on a schedule come next.</Empty>
      </section>

      <section class="list">
        <h2>Pull requests</h2>
        <Empty>Nothing to review. Pull requests will show here once dova is plugged in.</Empty>
      </section>

      <AgentCard />
    </div>
  );
}

function AgentCard() {
  return (
    <section class="list">
      <h2>wb-agent</h2>
      <Show
        when={status() !== 'down'}
        fallback={
          <p class="note note-error">{stopReason() ?? 'wb-agent stopped'}. Restart it from Ctrl K.</p>
        }
      >
        <ErrorNote error={health.error} />
        <Show when={!health.error && health()} fallback={<Empty>Starting…</Empty>}>
          {(h) => (
            <dl class="facts">
              <dt>Version</dt>
              <dd>{h().version}</dd>
              <dt>Runs in</dt>
              <dd>{h().distro ?? 'Linux, outside WSL'}</dd>
              <dt>Pi</dt>
              <dd>{h().pi ?? 'not found on PATH'}</dd>
              <dt>Pi’s files</dt>
              <dd class="mono">{h().agentDir}</dd>
            </dl>
          )}
        </Show>
      </Show>
    </section>
  );
}
