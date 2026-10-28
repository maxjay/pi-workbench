// Web tabs, as the app's own setting: they're Chromium inside the
// workbench, with your profile brought in from the Chromium you use now.
// Pi reaches them through a `browser` server the workbench adds to
// mcp.json; other Playwright servers keep their own browsers.

import { createSignal, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import { inApp } from '../../state/app';
import { addServer, browser } from '../../state/browser';
import { reload, setSection } from '../../state/settings';

/** Copies the profile of the Chromium you use now into the workbench's, on Windows. */
async function bringProfile() {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<string>('browser_import_profile');
}

export function BrowserSettings() {
  const add = createAction(async () => {
    await addServer();
    reload();
  });
  const [brought, setBrought] = createSignal<string>();
  const bring = createAction(async () => setBrought(await bringProfile()));
  const clear = createAction(async () => {
    if (!window.confirm('Forget every site’s cookies? You’ll be signed out of every site, and Pi with you.'))
      return;
    const { ask } = await import('../../agent');
    await ask({ method: 'browser.clearData' }, 'done');
  });
  const sessions = () => (browser.connections === 1 ? '1 session' : `${browser.connections} sessions`);

  return (
    <section class="settings-section">
      <header class="section-head">
        <div>
          <h1>Browser</h1>
          <p class="section-sub">
            Web tabs are Chromium, inside the workbench: your extensions, history and downloads. Manage
            extensions at <span class="mono">chrome://extensions</span>.
          </p>
        </div>
      </header>
      <Show when={!browser.connected}>
        <p class="note">
          {inApp
            ? 'The browser isn’t running yet. It starts with the app.'
            : 'In a plain browser, web tabs use the Chromium wb-agent reaches at WB_BROWSER (host:port).'}
          <Show when={browser.problem}> {browser.problem}</Show>
        </p>
      </Show>

      <div class="row">
        <div class="row-main">
          <span class="row-title">Your Chromium profile</span>
          <span class="row-detail">
            Bring in your extensions, sign-ins and history from the Chromium you use now. Close it first.
          </span>
        </div>
        <button
          type="button"
          class="button"
          disabled={!inApp || bring.busy()}
          onClick={() => void bring.run()}
        >
          {bring.busy() ? 'Bringing it in…' : 'Bring it in…'}
        </button>
      </div>
      <Show when={brought()}>{(said) => <p class="note">{said()}</p>}</Show>
      <ErrorNote error={bring.error()} />

      <div class="form-field">
        <span>Pi in the browser</span>
        <p class="section-sub">
          The workbench adds a Playwright MCP server called <span class="mono">browser</span> that uses these
          tabs. Any other Playwright server keeps its own browser.
        </p>
        <Show
          when={browser.server}
          fallback={
            <div class="check" data-ok="false">
              <span class="check-mark" aria-hidden="true">
                !
              </span>
              <span class="check-text">
                <span class="mono">browser</span> isn’t in MCP servers: Pi can’t use web tabs
              </span>
              <button type="button" class="button" disabled={add.busy()} onClick={() => void add.run()}>
                Add it back
              </button>
            </div>
          }
        >
          <div class="check" data-ok="true">
            <span class="check-mark" aria-hidden="true">
              ✓
            </span>
            <span class="check-text">
              <span class="mono">browser</span> is in MCP servers
            </span>
            <button type="button" class="button button-text" onClick={() => setSection('mcp')}>
              See it
            </button>
          </div>
          <Show when={browser.connections > 0}>
            <div class="check" data-ok="true">
              <span class="check-dot" aria-hidden="true" />
              <span class="check-text">Connected now: {sessions()}</span>
            </div>
          </Show>
        </Show>
        <ErrorNote error={add.error()} />
      </div>

      <div class="row">
        <div class="row-main">
          <span class="row-title">Sign-ins and site data</span>
          <span class="row-detail">Sites stay signed in between runs. Pi uses the same sign-ins.</span>
        </div>
        <button
          type="button"
          class="button"
          disabled={!browser.connected || clear.busy()}
          onClick={() => void clear.run()}
        >
          Clear site data…
        </button>
      </div>
      <ErrorNote error={clear.error()} />
    </section>
  );
}
