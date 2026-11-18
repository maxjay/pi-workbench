// Home. For now it only says what wbd found on this machine: the ask line,
// pull requests and workspaces arrive with the steps that make them work.

import { Show } from 'solid-js';
import { connect, status, stopReason } from '../daemon';
import { health } from '../state/health';

function Found(props: { name: string; version: string | null | undefined }) {
  return (
    <>
      <dt>{props.name}</dt>
      <dd classList={{ dim: !props.version }}>{props.version ?? 'not found'}</dd>
    </>
  );
}

export function Home() {
  return (
    <div class="home">
      <Show when={status() === 'down'}>
        <p class="home-down" role="alert">
          <span class="note-failed">{stopReason() ?? 'wbd stopped'}</span>
          <button type="button" class="button" onClick={() => void connect()}>
            Start it again
          </button>
        </p>
      </Show>
      <Show when={health()}>
        {(h) => (
          <section class="section" aria-labelledby="machine">
            <h2 id="machine">This machine</h2>
            <dl class="facts">
              <Found name="wbd" version={h().version} />
              <dt>Runs in</dt>
              <dd>{h().distro ?? 'Linux, outside WSL'}</dd>
              <Found name="Pi" version={h().pi} />
              <Found name="Claude Code" version={h().claude} />
              <Found name="dova" version={h().dova} />
            </dl>
          </section>
        )}
      </Show>
    </div>
  );
}
