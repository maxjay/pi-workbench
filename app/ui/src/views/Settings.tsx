// Pi's own settings, the same ones every session uses: one AGENTS.md, one
// mcp.json, one model registry. No per-codespace copies.

import { For, Match, Switch } from 'solid-js';
import { health } from '../state/health';
import { reload, section, sections, setSection } from '../state/settings';
import { Background } from './settings/Background';
import { Instructions } from './settings/Instructions';
import { Mcp } from './settings/Mcp';
import { Models } from './settings/Models';

export function Settings() {
  return (
    <div class="page settings">
      <nav class="settings-nav" aria-label="Settings">
        <For each={sections}>
          {(s) => (
            <button
              type="button"
              class="settings-link"
              aria-current={section() === s.id ? 'page' : undefined}
              onClick={() => setSection(s.id)}
            >
              <span>{s.title}</span>
              <span class="settings-file">{s.file}</span>
            </button>
          )}
        </For>
        {/* The app's own settings, apart from Pi's. */}
        <div class="settings-line" />
        <button
          type="button"
          class="settings-link"
          aria-current={section() === 'background' ? 'page' : undefined}
          onClick={() => setSection('background')}
        >
          <span>Background</span>
          <span class="settings-file">tray, sign-in</span>
        </button>
        <div class="settings-foot">
          <p class="mono">{health()?.agentDir ?? ''}</p>
          <button type="button" class="button" onClick={reload}>
            Reload files
          </button>
        </div>
      </nav>
      <div class="settings-body">
        <Switch>
          <Match when={section() === 'instructions'}>
            <Instructions />
          </Match>
          <Match when={section() === 'mcp'}>
            <Mcp />
          </Match>
          <Match when={section() === 'models'}>
            <Models />
          </Match>
          <Match when={section() === 'background'}>
            <Background />
          </Match>
        </Switch>
      </div>
    </div>
  );
}
