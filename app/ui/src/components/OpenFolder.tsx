// Open folder: a folder in WSL becomes a codespace. You type a path; the
// folders in the one you've typed so far are listed as you go, git ones
// marked, and Tab or a click goes into one. Enter opens it.

import { createResource, createSignal, For, Show } from 'solid-js';
import { codespaces, foldersIn, openFolder } from '../state/codespaces';
import { open as openTab } from '../state/tabs';
import { createAction } from './action';
import { Icon } from './Icon';
import { ErrorNote } from './Note';

const [shown, setShown] = createSignal(false);

export function openFolderDialog() {
  setShown(true);
}

export function OpenFolder() {
  return (
    <Show when={shown()}>
      <Dialog />
    </Show>
  );
}

function Dialog() {
  // Start beside the codespace opened last, where the next one most likely is.
  const last = codespaces()[0]?.space.path;
  const [typed, setTyped] = createSignal(last ? `${last.slice(0, last.lastIndexOf('/') + 1)}` : '~/');
  const [at, setAt] = createSignal(0);
  let input!: HTMLInputElement;
  const dir = () => typed().slice(0, typed().lastIndexOf('/') + 1) || '~/';
  const part = () =>
    typed()
      .slice(typed().lastIndexOf('/') + 1)
      .toLowerCase();
  const [listed] = createResource(dir, (d) => foldersIn(d).catch(() => undefined));
  const matching = () =>
    (listed.latest?.folders ?? []).filter((f) => f.name.toLowerCase().startsWith(part()));
  const target = () => {
    const pick = matching()[Math.min(at(), matching().length - 1)];
    return part() && pick ? `${dir()}${pick.name}` : typed().replace(/\/+$/, '') || '~';
  };
  const into = (name: string) => {
    setTyped(`${dir()}${name}/`);
    setAt(0);
    input.focus();
  };
  const go = createAction(async () => {
    const view = await openFolder(target());
    setShown(false);
    openTab(`space:${view.space.id}`);
  });
  const close = () => setShown(false);

  return (
    <>
      <div class="dialog-backdrop" aria-hidden="true" onClick={close} />
      <div class="dialog open-folder" role="dialog" aria-label="Open a folder">
        <div class="dialog-head">
          <h2>Open a folder</h2>
          <p>A folder in WSL becomes a codespace: its files, terminals and sessions with Pi.</p>
        </div>
        <label class="folder-path">
          <Icon name="folder" size={14} />
          <span class="visually-hidden">Folder</span>
          <input
            ref={(el) => {
              input = el;
              queueMicrotask(() => {
                el.focus();
                el.setSelectionRange(el.value.length, el.value.length);
              });
            }}
            class="mono"
            spellcheck={false}
            value={typed()}
            onInput={(e) => {
              setTyped(e.currentTarget.value);
              setAt(0);
            }}
            onKeyDown={(e) => {
              const list = matching();
              if (e.key === 'ArrowDown') setAt(Math.min(at() + 1, list.length - 1));
              else if (e.key === 'ArrowUp') setAt(Math.max(at() - 1, 0));
              else if (e.key === 'Tab' && list.length > 0) into(list[Math.min(at(), list.length - 1)].name);
              else if (e.key === 'Enter') void go.run();
              else if (e.key === 'Escape') close();
              else return;
              e.preventDefault();
              e.stopPropagation();
            }}
          />
        </label>
        <div class="folder-list" role="listbox" aria-label="Folders">
          <For each={matching()}>
            {(folder, i) => (
              <button
                type="button"
                role="option"
                class="folder-row"
                aria-selected={i() === at()}
                onMouseMove={() => setAt(i())}
                onClick={() => into(folder.name)}
              >
                <span class="folder-icon" data-git={folder.branch !== null}>
                  <Icon name="folder" size={15} />
                </span>
                <span class="mono">{folder.name}/</span>
                <span class="folder-detail">{folder.branch ? `git · ${folder.branch}` : ''}</span>
                <Show when={folder.codespace}>
                  <span class="folder-tag">A codespace already</span>
                </Show>
              </button>
            )}
          </For>
          <Show when={!listed.loading && matching().length === 0}>
            <p class="folder-none">
              {listed.latest
                ? part()
                  ? `Nothing here starts with “${part()}”`
                  : 'No folders in here'
                : `No folder at ${dir()}`}
            </p>
          </Show>
        </div>
        <ErrorNote error={go.error()} />
        <div class="dialog-foot">
          <span class="mono">Opens {target()} in WSL</span>
          <button type="button" class="button button-text" onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            class="button button-primary"
            disabled={go.busy()}
            onClick={() => void go.run()}
          >
            Open
          </button>
        </div>
      </div>
    </>
  );
}
