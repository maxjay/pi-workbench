// The app around the page, on Windows: its own settings for running in
// the background, and the tray, which this keeps in step with the tasks.
// In a plain browser (`npm run dev`) there's neither.

import { createEffect, createResource, createRoot } from 'solid-js';
import { allPaused, going, pauseAll, tasks } from './tasks';

export const inApp = '__TAURI_INTERNALS__' in window;

export interface BackgroundSettings {
  startAtSignIn: boolean;
  /** Closing the window keeps the app in the tray, rather than quitting. */
  closeToTray: boolean;
}

async function invoke<T>(command: string, args?: Record<string, unknown>) {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(command, args);
}

const [background, { mutate }] = createRoot(() =>
  createResource(
    () => inApp,
    () => invoke<BackgroundSettings>('background_get'),
  ),
);

export { background };

export async function setBackground(settings: BackgroundSettings) {
  await invoke('background_set', { settings });
  mutate(settings);
}

if (inApp) {
  createRoot(() => {
    // The tray says how many tasks wait on you, and whether they're paused.
    createEffect(() => {
      const waiting = tasks().filter((view) => going(view)?.state === 'asking').length;
      void invoke('tray_update', { waiting, paused: allPaused() }).catch(() => {});
    });

    // A run that fails or comes to need you. Windows notifications for
    // these come later; the app does nothing with them yet.
    const last = new Map<string, string>();
    createEffect(() => {
      for (const view of tasks()) {
        for (const run of view.runs.slice(0, 1)) {
          const was = last.get(run.id);
          last.set(run.id, run.state);
          if (was !== undefined && was !== run.state && (run.state === 'asking' || run.state === 'failed'))
            void invoke('notify_run', { task: view.task.name, state: run.state }).catch(() => {});
        }
      }
    });
  });

  void import('@tauri-apps/api/event').then(({ listen }) =>
    listen<string>('tray', (event) => {
      if (event.payload === 'pauseAll') void pauseAll(!allPaused());
    }),
  );
}
