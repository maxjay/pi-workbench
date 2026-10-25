// How the app keeps running when you're not looking at it, as tasks only
// run while it does: starting at sign-in, what closing the window does,
// and what the tray offers. These are the app's own, not Pi's.

import { For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { ErrorNote } from '../../components/Note';
import { Switch } from '../../components/Switch';
import { type BackgroundSettings, background, inApp, setBackground } from '../../state/app';
import { allPaused, going, tasks } from '../../state/tasks';

export function Background() {
  const change = createAction((next: BackgroundSettings) => setBackground(next));
  const current = () => background.latest ?? { startAtSignIn: true, closeToTray: true };
  const waiting = () => tasks().filter((view) => going(view)?.state === 'asking').length;

  return (
    <section class="settings-section">
      <header class="section-head">
        <div>
          <h1>Background</h1>
          <p class="section-sub">
            Tasks only run while the app does. These keep it running when you’re not looking at it.
          </p>
        </div>
      </header>
      <Show when={!inApp}>
        <p class="note">These apply to the app on Windows. A browser has no tray or sign-in to start with.</p>
      </Show>
      <ErrorNote error={change.error() ?? background.error} />
      <div class="row">
        <div class="row-main">
          <span class="row-title">Start when I sign in</span>
          <span class="row-detail">Opens in the tray, without a window, so tasks run on time.</span>
        </div>
        <Switch
          on={current().startAtSignIn}
          label="Start when I sign in"
          onChange={(on) => inApp && void change.run({ ...current(), startAtSignIn: on })}
        />
      </div>
      <fieldset class="form-field">
        <legend>When I close the window</legend>
        <For
          each={
            [
              [
                true,
                'Keep running in the tray',
                'Tasks carry on. Open the window again from the π in the tray.',
              ],
              [
                false,
                'Quit',
                'Nothing runs until you open the app again. Scheduled runs it misses are run when it starts.',
              ],
            ] as [boolean, string, string][]
          }
        >
          {([value, label, detail]) => (
            <button
              type="button"
              class="choice"
              aria-pressed={current().closeToTray === value}
              disabled={!inApp || change.busy()}
              onClick={() => void change.run({ ...current(), closeToTray: value })}
            >
              <span class="choice-ring" aria-hidden="true" />
              <span class="choice-text">
                <span>{label}</span>
                <span class="count">{detail}</span>
              </span>
            </button>
          )}
        </For>
      </fieldset>
      <div class="form-field">
        <span>In the tray</span>
        <p class="section-sub">Click the π icon to open the window. Right-click it for this:</p>
        <div class="tray-sketch" aria-hidden="true">
          <span class="tray-sketch-first">Open Pi Workbench</span>
          <Show when={waiting() > 0}>
            <span class="tray-sketch-waiting">
              {waiting() === 1 ? '1 task needs you' : `${waiting()} tasks need you`}
            </span>
          </Show>
          <span class="tray-sketch-line" />
          <span>{allPaused() ? 'Resume tasks' : 'Pause all tasks'}</span>
          <span class="tray-sketch-line" />
          <span>Quit</span>
        </div>
        <p class="count">Quit stops Pi everywhere, tasks included, until you open the app again.</p>
      </div>
    </section>
  );
}
