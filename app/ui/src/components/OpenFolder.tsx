// Picking a folder in WSL to open as a workspace: go into folders until you
// are in the one you want, then open it. Or type its path.

import { createResource, createSignal, For, Show } from 'solid-js';
import { ask } from '../daemon';
import { openFolder } from '../state/workspaces';
import { createAction } from './action';

const [dir, setDir] = createSignal<string>();

/** Opens the dialog, in your home folder. */
export const openFolderDialog = () => setDir('~');

export function OpenFolder() {
  const [listing] = createResource(dir, (d) =>
    ask({ method: 'folders.list', params: { dir: d } }, 'folders'),
  );
  const opening = createAction(async (path: string) => {
    await openFolder(path);
    setDir();
  });
  /** The folder above the one listed. */
  const parent = () => {
    const here = listing.latest?.dir;
    return here && here !== '/' ? here.slice(0, here.lastIndexOf('/')) || '/' : undefined;
  };

  return (
    <Show when={dir()}>
      <div class="scrim">
        <button type="button" class="scrim-close" aria-label="Close" onClick={() => setDir()} />
        <div class="palette" role="dialog" aria-label="Open a folder">
          <form
            class="folder-path"
            onSubmit={(e) => {
              e.preventDefault();
              setDir(new FormData(e.currentTarget).get('path') as string);
            }}
          >
            <label for="folder-path" class="dim">
              In
            </label>
            <input id="folder-path" name="path" class="palette-input" value={listing.latest?.dir ?? dir()} />
          </form>
          <div class="palette-rows">
            <Show when={listing.error}>
              <p class="palette-none note-failed">{String(listing.error.message ?? listing.error)}</p>
            </Show>
            <Show when={parent()}>
              {(up) => (
                <button type="button" class="row palette-row" onClick={() => setDir(up())}>
                  <span class="palette-title">..</span>
                  <span class="palette-detail dim">{up()}</span>
                </button>
              )}
            </Show>
            <For each={listing.latest?.folders}>
              {(folder) => (
                <button type="button" class="row palette-row" onClick={() => setDir(folder.path)}>
                  <span class="palette-title">{folder.name}/</span>
                  <span class="palette-detail dim">{folder.branch}</span>
                </button>
              )}
            </For>
          </div>
          <div class="folder-foot">
            <Show when={opening.error()}>{(error) => <span class="note-failed">{error()}</span>}</Show>
            <span class="top-gap" />
            <button type="button" class="button" onClick={() => setDir()}>
              Cancel
            </button>
            <button
              type="button"
              class="button button-primary"
              disabled={!listing.latest || opening.busy()}
              onClick={() => listing.latest && void opening.run(listing.latest.dir)}
            >
              Open this folder
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
