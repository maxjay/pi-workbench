// A workspace's tab. On the left a list: Files or Changes. In the middle the
// one thing you picked, with the terminals under it. On the right the agent.
// While nothing is open in the middle, the agent has the middle.
//
// A workspace is one repo, or a folder that holds several: each a folder
// inside it, a worktree of one of your clones on a branch of its own. Changes
// then lists each repo by itself, and Add repo puts one more in.

import { createMemo, createResource, For, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../components/action';
import { openPalette } from '../components/Palette';
import { ask } from '../daemon';
import type { GitMark } from '../protocol/GitMark';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { buffers, checkDisk, forgetBuffer } from '../state/editor';
import { allFiles, readFolder, refreshGit, refreshRepo, repos } from '../state/repo';
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
  shortPath,
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
  const absolute = (relative: string) => `${root}/${relative}`;
  const relative = (path: string) => path.slice(root.length + 1);

  // A workspace that is not a repo itself takes repos: the folders directly inside it that
  // are one. They are looked for again whenever the folders in it change.
  const takes = props.view.branch === null;
  const [inside] = createResource(
    () => (takes ? `in:${top()?.folders.join('/') ?? ''}` : undefined),
    async () => (await ask({ method: 'workspace.repos', params: { dir: root } }, 'folders')).folders,
  );
  /** Where git is asked: each repo inside, or the workspace's own folder. */
  const parts = createMemo(() => {
    const found = inside.latest ?? [];
    return found.length > 0
      ? found.map((repo) => ({ name: repo.name, path: repo.path }))
      : [{ name: '', path: root }];
  });
  const refresh = () => {
    for (const part of parts()) if (part.name) void refreshGit(part.path);
    return refreshRepo(root, ui().expanded);
  };

  const adding = createAction(async (clone: string) => {
    await ask({ method: 'workspace.add', params: { dir: root, clone, branch: null } }, 'done');
    await refresh();
  });
  /** Lists your clones, to pick the one to put in. */
  const pickRepo = createAction(async () => {
    const { dir, folders } = await ask({ method: 'repos.list' }, 'folders');
    const here = new Set(parts().map((part) => part.name));
    openPalette(
      folders
        .filter((clone) => !here.has(clone.name))
        .map((clone) => ({
          group: 'Repo',
          title: clone.name,
          detail: shortPath(clone.path),
          run: () => void adding.run(clone.path),
        })),
      `Add a repo from ${shortPath(dir)}`,
    );
  });

  /** The file shown, when the tab shown is a file. */
  const file = () => {
    const tab = ui().current;
    return tab && isFile(tab) ? tab : undefined;
  };
  /** Each changed file by its path in the workspace, in whichever repo it is. */
  const changed = createMemo(() => {
    const all = new Map<string, GitMark>();
    for (const part of parts())
      for (const f of repos[part.path]?.git?.files ?? [])
        all.set(part.name ? `${part.name}/${f.path}` : f.path, f.mark);
    return all;
  });

  const editor = createEditor({
    path: file,
    gitKey: () => JSON.stringify(parts().map((part) => repos[part.path]?.git?.summary ?? null)),
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
            <Show when={changed().size}>{(n) => <span class="dim">{n()}</span>}</Show>
          </button>
          <Show when={takes}>
            <span class="top-gap" />
            <button
              type="button"
              class="strip-tab"
              title="Puts a worktree of one of your clones into this workspace, on a branch of its own"
              onClick={() => void pickRepo.run()}
            >
              Add repo
            </button>
          </Show>
        </div>
        <div class="left-body">
          <Show when={adding.busy()}>
            <p class="left-note dim">Adding the repo</p>
          </Show>
          <Show when={adding.error() ?? pickRepo.error()}>
            {(error) => <p class="left-note note-failed">{error()}</p>}
          </Show>
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
            <For each={parts()}>
              {(part) => (
                <>
                  <Show when={part.name}>
                    <p class="left-group">{part.name}</p>
                  </Show>
                  <Show
                    when={repos[part.path]?.git}
                    fallback={
                      <p class="left-note dim">
                        {repos[part.path]?.git === undefined
                          ? 'Asking git'
                          : takes
                            ? 'No repo in this workspace yet.'
                            : 'Not a git folder.'}
                      </p>
                    }
                  >
                    {(git) => (
                      <>
                        <p class="left-note dim">
                          {git().summary.branch} · +{git().summary.added} -{git().summary.removed}
                        </p>
                        <BranchPull
                          dir={part.path}
                          onNew={() => openTab(id, part.name ? `newpr:${part.path}` : 'newpr')}
                          onOpen={(pr) => open(`pr:${part.name ? part.path : id}:${pr}`)}
                        />
                        <For each={git().files} fallback={<p class="left-note dim">Nothing changed.</p>}>
                          {(f) => (
                            <button
                              type="button"
                              class="row tree-row"
                              aria-current={file() === `${part.path}/${f.path}` ? 'true' : undefined}
                              onClick={() => openTab(id, `${part.path}/${f.path}`)}
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
                </>
              )}
            </For>
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
        <Show when={ui().current?.startsWith('newpr') && ui().current} keyed>
          {(tab) => (
            <NewPull dir={tab === 'newpr' ? root : tab.slice('newpr:'.length)} onDone={() => close(tab)} />
          )}
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
