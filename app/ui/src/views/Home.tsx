// The first tab: ask Pi something, and see codespaces, tasks and pull
// requests at a glance. Only the shell is here yet; each list fills in as
// its stage lands.

import { Show } from 'solid-js';
import { status, stopReason } from '../agent';
import { Empty, ErrorNote } from '../components/Note';
import { health } from '../state/health';

export function Home() {
  return (
    <div class="page home">
      <section class="ask">
        <h1>What are we working on?</h1>
        <textarea
          class="ask-box"
          rows={3}
          disabled
          placeholder="Ask Pi anything. The scratchpad arrives in the next build."
        />
      </section>

      <section class="list">
        <h2>Codespaces</h2>
        <Empty>No codespaces yet. Opening a folder in WSL as a codespace comes in stage 3.</Empty>
      </section>

      <section class="list">
        <h2>Tasks</h2>
        <Empty>No tasks yet. Tasks come with the scratchpad.</Empty>
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
