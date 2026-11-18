// Home: your workspaces, and what wbd found on this machine. The ask line
// and pull requests arrive with the steps that make them work.

import { For, Show } from 'solid-js';
import { Glyph } from '../components/Glyph';
import { openFolderDialog } from '../components/OpenFolder';
import { connect, status, stopReason } from '../daemon';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { health } from '../state/health';
import { open } from '../state/tabs';
import { workspaces } from '../state/workspaces';

function Found(props: { name: string; version: string | null | undefined }) {
  return (
    <>
      <dt>{props.name}</dt>
      <dd classList={{ dim: !props.version }}>{props.version ?? 'not found'}</dd>
    </>
  );
}

/** Where a workspace's git stands, in a few words. */
function standing(view: WorkspaceView) {
  if (!view.exists) return 'not there any more';
  if (!view.git) return 'not a git folder';
  return `${view.git.branch} · ${view.git.files === 0 ? 'clean' : `${view.git.files} changed`}`;
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

      <section class="section" aria-labelledby="workspaces">
        <div class="section-head">
          <h2 id="workspaces">Workspaces</h2>
          <span class="top-gap" />
          <button type="button" class="button" disabled={status() !== 'ready'} onClick={openFolderDialog}>
            Open folder
          </button>
        </div>
        <div class="rows">
          <For
            each={workspaces()}
            fallback={<p class="rows-none dim">None yet. Open a folder in WSL to start one.</p>}
          >
            {(view) => (
              <button type="button" class="row workspace-row" onClick={() => open(`ws:${view.workspace.id}`)}>
                <Glyph name={view.workspace.name} size={12} />
                <span class="workspace-name">{view.workspace.name}</span>
                <span class="dim">{standing(view)}</span>
                <span class="workspace-path dim">{view.workspace.path}</span>
              </button>
            )}
          </For>
        </div>
      </section>

      <Show when={health()}>
        {(h) => (
          <section class="section" aria-labelledby="machine">
            <div class="section-head">
              <h2 id="machine">This machine</h2>
            </div>
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
