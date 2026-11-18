// A workspace's tab. On the left a list: Files or Changes. In the middle the
// one thing you picked, with the terminals under it. On the right the agent.
// While nothing is open in the middle, the agent has the middle.

import { createMemo, For, onCleanup, onMount, Show } from 'solid-js';
import { openPalette } from '../components/Palette';
import type { GitMark } from '../protocol/GitMark';
import type { WorkspaceView } from '../protocol/WorkspaceView';
import { defaultAgent, send, start } from '../state/conversations';
import { buffers, checkDisk, forgetBuffer } from '../state/editor';
import { refreshRepo, repos } from '../state/repo';
import { forget, workspaceById } from '../state/workspaces';
import { Chat } from './workspace/Chat';
import { createEditor } from './workspace/Editor';
import { LogView } from './workspace/LogView';
import { Pull } from './workspace/Pull';
import { Terminals } from './workspace/Terminals';
import { ThreadView } from './workspace/ThreadView';
import { marks, Tree } from './workspace/Tree';
import { closeTab, isFile, labelOf, nameOf, openTab, setConversation, setLeft, uiOf } from './workspace/ui';

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
    onSaved: () => void refreshRepo(root),
  });

  /** Opens a file of this workspace at a line. */
  const openAt = (path: string, line: number | null) => {
    const full = absolute(path.replace(/^\//, ''));
    if (line) editor.goToLine(full, line);
    openTab(id, full);
  };
  /** Says something to the agent here: in the conversation shown, or a new one. */
  const tell = async (text: string) => {
    const shown = ui().conversation;
    if (shown) await send(shown, text);
    else setConversation(id, await start(defaultAgent(), root, text));
  };
  /** The ids in a tab that isn't a file: `log:6` is [6], `thread:1:4` is [1, 4]. */
  const ids = (tab: string) => tab.split(':').slice(1).map(Number);

  const close = (tab: string) => {
    closeTab(id, tab);
    if (isFile(tab)) {
      editor.drop(tab);
      forgetBuffer(tab);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey) || e.key !== 'p') return;
    e.preventDefault();
    openPalette(
      (repo()?.files ?? []).map((path) => ({
        group: 'Files',
        title: nameOf(path),
        detail: path,
        run: () => openTab(id, absolute(path)),
      })),
      'Find a file',
    );
  };

  onMount(() => {
    void refreshRepo(root);
    // The folder and git are asked again every few seconds, and the open
    // files checked against the disk more often than that.
    const repoTimer = setInterval(() => document.hidden || void refreshRepo(root), 5000);
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
          <Show when={repo()?.git}>
            <button
              type="button"
              class="strip-tab"
              aria-pressed={ui().left === 'pull'}
              onClick={() => setLeft(id, 'pull')}
            >
              Pull request
            </button>
          </Show>
        </div>
        <div class="left-body">
          <Show when={ui().left === 'files'}>
            <Show when={repo()} fallback={<p class="left-note dim">Reading the folder</p>}>
              {(r) => (
                <>
                  <Tree
                    id={id}
                    files={r().files}
                    expanded={ui().expanded}
                    changed={changed()}
                    current={file() ? relative(file() as string) : undefined}
                    onOpen={(path) => openTab(id, absolute(path))}
                  />
                  <Show when={r().files.length === 0}>
                    <p class="left-note dim">This folder is empty.</p>
                  </Show>
                  <Show when={r().truncated}>
                    <p class="left-note dim">Only the first {r().files.length} files are listed.</p>
                  </Show>
                </>
              )}
            </Show>
          </Show>
          <Show when={ui().left === 'changes'}>
            <Show when={repo()?.git} fallback={<p class="left-note dim">Not a git folder.</p>}>
              {(git) => (
                <>
                  <p class="left-note dim">
                    {git().summary.branch} · +{git().summary.added} -{git().summary.removed}
                  </p>
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
          <Show when={ui().left === 'pull'}>
            <Pull
              dir={root}
              pr={ui().pull}
              current={ui().current}
              onLog={(run) => openTab(id, `log:${run}`)}
              onThread={(pr, thread) => openTab(id, `thread:${pr}:${thread}`)}
            />
          </Show>
        </div>
      </aside>

      <section class="middle" aria-label="Open file">
        <Show when={ui().tabs.length > 0}>
          <div class="bar strip">
            <For each={ui().tabs}>
              {(tab) => (
                <span class="strip-tab strip-tab-closable" data-current={ui().current === tab}>
                  <button type="button" class="strip-open" title={tab} onClick={() => openTab(id, tab)}>
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
        <Show when={ui().current?.startsWith('log:') && ui().current} keyed>
          {(tab) => (
            <LogView
              dir={root}
              run={ids(tab)[0]}
              files={repo()?.files ?? []}
              onOpenAt={openAt}
              onAsk={(text) => void tell(text)}
            />
          )}
        </Show>
        <Show when={ui().current?.startsWith('thread:') && ui().current} keyed>
          {(tab) => <ThreadView dir={root} pr={ids(tab)[0]} thread={ids(tab)[1]} onOpenAt={openAt} />}
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
