// A workspace's tab. On the left a list: Files or Changes. In the middle the
// one thing you picked, with the terminals under it. On the right the agent.
// While nothing is open in the middle, the agent has the middle.

import { createMemo, For, onCleanup, onMount, Show } from 'solid-js';
import { openPalette } from '../components/Palette';
import type { GitMark } from '../protocol/GitMark';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { buffers, checkDisk, forgetBuffer } from '../state/editor';
import { allFiles, readFolder, refreshRepo, repos } from '../state/repo';
import { open } from '../state/tabs';
import { forget, workspaceById } from '../state/workspaces';
import { Chat } from './workspace/Chat';
import { createEditor } from './workspace/Editor';
import { NewPull } from './workspace/NewPull';
import { BranchPull } from './workspace/Pull';
import { Terminals } from './workspace/Terminals';
import { marks, Tree } from './workspace/Tree';
import {
  closeTab,
  isFile,
  labelOf,
  nameOf,
  openTab,
  setConversation,
  setLeft,
  toggleFolder,
  uiOf,
} from './workspace/ui';

export function WorkspacePage(props: { id: string }) {
  return (
    <Show when={workspaceById(props.id)} keyed>
      {(view) => (
        <Show when={view.exists} fallback={<Missing view={view} />}>
          <Page view={view} />
        </Show>
      )}
    </Show>
  );
}

function Missing(props: { view: WorkspaceView }) {
  return (
    <div class="gone">
      <h1>{props.view.workspace.name} is not there any more</h1>
      <p class="dim">{props.view.workspace.path}</p>
      <p>The folder was moved or deleted.</p>
      <button type="button" class="button" onClick={() => void forget(props.view.workspace.id)}>
        Take it off the list
      </button>
    </div>
  );
}

function Page(props: { view: WorkspaceView }) {
  const id = props.view.workspace.id;
  const root = props.view.workspace.path;
  const ui = () => uiOf(id);
  const repo = () => repos[root];
  /** The workspace's own folder, as last read. */
  const top = () => repo()?.folders[''];
  const refresh = () => refreshRepo(root, ui().expanded);
  const absolute = (relative: string) => `${root}/${relative}`;
  const relative = (path: string) => path.slice(root.length + 1);

  /** The file shown, when the tab shown is a file. */
  const file = () => {
    const tab = ui().current;
    return tab && isFile(tab) ? tab : undefined;
  };
  const changed = createMemo(
    () => new Map<string, GitMark>((repo()?.git?.files ?? []).map((f) => [f.path, f.mark])),
  );

  const editor = createEditor({
    path: file,
    gitKey: () => JSON.stringify(repo()?.git?.summary ?? null),
    onSaved: () => void refresh(),
  });

  const close = (tab: string) => {
    closeTab(id, tab);
    if (isFile(tab)) {
      editor.drop(tab);
      forgetBuffer(tab);
    }
  };

  /** Opens a folder in the tree and reads it at once, or shuts it. */
  const toggle = (folder: string) => {
    toggleFolder(id, folder);
    if (ui().expanded.includes(folder)) void readFolder(root, folder);
  };

  // Ctrl P asks for every file only now: in a large repository that is a long list.
  const onKey = async (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey) || e.key !== 'p') return;
    e.preventDefault();
    const { files, truncated } = await allFiles(root);
    openPalette(
      files.map((path) => ({
        group: 'Files',
        title: nameOf(path),
        detail: path,
        run: () => openTab(id, absolute(path)),
      })),
      truncated ? `Find a file (the first ${files.length.toLocaleString('en-US')})` : 'Find a file',
    );
  };

  onMount(() => {
    void refresh();
    // The open folders and git are asked again every few seconds, and the
    // open files checked against the disk more often than that.
    const repoTimer = setInterval(() => document.hidden || void refresh(), 5000);
    const diskTimer = setInterval(async () => {
      if (document.hidden) return;
      const fresh = await checkDisk(ui().tabs.filter(isFile)).catch(() => new Map<string, string>());
      for (const [path, text] of fresh) editor.replace(path, text);
    }, 1500);
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      clearInterval(repoTimer);
      clearInterval(diskTimer);
      window.removeEventListener('keydown', onKey);
    });
  });

  return (
    <div class="columns" data-chat={ui().tabs.length === 0 ? 'middle' : 'right'}>
      <aside class="left" aria-label="Lists">
        <div class="bar strip">
          <button
            type="button"
            class="strip-tab"
            aria-pressed={ui().left === 'files'}
            onClick={() => setLeft(id, 'files')}
          >
            Files
          </button>
          <button
            type="button"
            class="strip-tab"
            aria-pressed={ui().left === 'changes'}
            onClick={() => setLeft(id, 'changes')}
          >
            Changes
            <Show when={repo()?.git?.summary.files}>{(n) => <span class="dim">{n()}</span>}</Show>
          </button>
        </div>
        <div class="left-body">
          <Show when={ui().left === 'files'}>
            <Show when={repo()} fallback={<p class="left-note dim">Reading the folder</p>}>
              {(r) => (
                <>
                  <Tree
                    folders={r().folders}
                    expanded={ui().expanded}
                    changed={changed()}
                    current={file() ? relative(file() as string) : undefined}
                    onOpen={(path) => openTab(id, absolute(path))}
                    onToggle={toggle}
                  />
                  <Show when={!top()}>
                    <p class="left-note dim">This folder could not be read.</p>
                  </Show>
                  <Show when={top()?.folders.length === 0 && top()?.files.length === 0}>
                    <p class="left-note dim">This folder is empty.</p>
                  </Show>
                </>
              )}
            </Show>
          </Show>
          <Show when={ui().left === 'changes'}>
            <Show
              when={repo()?.git}
              fallback={
                <p class="left-note dim">{repo()?.git === undefined ? 'Asking git' : 'Not a git folder.'}</p>
              }
            >
              {(git) => (
                <>
                  <p class="left-note dim">
                    {git().summary.branch} · +{git().summary.added} -{git().summary.removed}
                  </p>
                  <BranchPull
                    dir={root}
                    onNew={() => openTab(id, 'newpr')}
                    onOpen={(pr) => open(`pr:${id}:${pr}`)}
                  />
                  <For each={git().files} fallback={<p class="left-note dim">Nothing changed.</p>}>
                    {(f) => (
                      <button
                        type="button"
                        class="row tree-row"
                        aria-current={file() === absolute(f.path) ? 'true' : undefined}
                        onClick={() => openTab(id, absolute(f.path))}
                      >
                        <span class="tree-name">{f.path}</span>
                        <span class="mark" data-state={marks[f.mark].state}>
                          {marks[f.mark].letter}
                        </span>
                      </button>
                    )}
                  </For>
                </>
              )}
            </Show>
          </Show>
        </div>
      </aside>

      <section class="middle" aria-label="Open file">
        <Show when={ui().tabs.length > 0}>
          <div class="bar strip">
            <For each={ui().tabs}>
              {(tab) => (
                <span class="strip-tab strip-tab-closable" data-current={ui().current === tab}>
                  <button
                    type="button"
                    class="strip-open"
                    title={isFile(tab) ? tab : undefined}
                    onClick={() => openTab(id, tab)}
                  >
                    {labelOf(tab)}
                    <Show when={buffers[tab]?.dirty}>
                      <span role="img" aria-label="Not saved">
                        {' '}
                        •
                      </span>
                    </Show>
                  </button>
                  <button
                    type="button"
                    class="strip-close"
                    aria-label={`Close ${labelOf(tab)}`}
                    onClick={() => close(tab)}
                  >
                    ×
                  </button>
                </span>
              )}
            </For>
          </div>
        </Show>
        <Show when={ui().current === 'newpr'}>
          <NewPull dir={root} onDone={() => close('newpr')} />
        </Show>
        {editor.view}
      </section>
      <Terminals dir={root} />
      <Chat
        cwd={root}
        shown={ui().conversation}
        onShow={(conversation) => setConversation(id, conversation)}
        onOpenFile={(path) => openTab(id, path)}
      />
    </div>
  );
}
