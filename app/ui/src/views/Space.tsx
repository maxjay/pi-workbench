// A codespace's tab: its files down the left; the editor, with the
// terminals under it, in the middle; Pi on the right. Ctrl P finds a file,
// a command, a line or a session.

import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { onEvent } from '../agent';
import { createAction } from '../components/action';
import { Icon } from '../components/Icon';
import { ErrorNote } from '../components/Note';
import type { CodespaceView } from '../protocol/CodespaceView';
import { forget, spaceById } from '../state/codespaces';
import { buffers, checkDisk, forgetBuffer } from '../state/editor';
import { refreshRepo, repos } from '../state/repo';
import { live } from '../state/sessions';
import { close as closeTab } from '../state/tabs';
import { openTerminal } from '../state/terminals';
import { writes } from './chat/tools';
import { createEditor } from './space/Editor';
import { integrations } from './space/integrations';
import { type QuickItem, QuickOpen } from './space/QuickOpen';
import { Sessions } from './space/Sessions';
import { Terminals } from './space/Terminals';
import { Tree } from './space/Tree';
import { closeFile, openFile, setUi, showSession, uiOf } from './space/ui';

export function SpacePage(props: { id: string }) {
  return (
    <Show when={spaceById(props.id)} keyed fallback={<div class="page space-page" />}>
      {(view) => (
        <Show when={view.exists} fallback={<Missing view={view} />}>
          <Page id={props.id} />
        </Show>
      )}
    </Show>
  );
}

function Missing(props: { view: CodespaceView }) {
  const drop = createAction(async () => {
    await forget(props.view.space.id);
    closeTab(`space:${props.view.space.id}`);
  });
  return (
    <div class="page space-page">
      <div class="space-missing">
        <h1>{props.view.space.name} isn’t there any more</h1>
        <p class="mono">{props.view.space.path}</p>
        <p>The folder was moved or deleted. Its sessions are still in Pi’s files.</p>
        <button type="button" class="button" disabled={drop.busy()} onClick={() => void drop.run()}>
          Forget this codespace
        </button>
        <ErrorNote error={drop.error()} />
      </div>
    </div>
  );
}

function Page(props: { id: string }) {
  const view = () => spaceById(props.id) as CodespaceView;
  const root = view().space.path;
  const ui = () => uiOf(props.id);
  const repo = () => repos[root];
  const [quick, setQuick] = createSignal<string>();
  const [recent, setRecent] = createSignal<string[]>([]);
  const [saves, setSaves] = createSignal(0);

  /** Live sessions in this codespace. */
  const here = () =>
    Object.values(live).filter((s) => s.state.cwd === root || s.state.cwd.startsWith(`${root}/`));
  /** Files Pi is writing right now. */
  const writing = createMemo(() => {
    const set = new Set<string>();
    for (const s of here())
      for (const entry of s.entries)
        if (entry.kind === 'tool' && entry.status === 'running') {
          const path = writes(entry, s.state.cwd);
          if (path) set.add(path);
        }
    return set;
  });
  const byPi = (path: string) => here().some((s) => s.state.files.includes(path));

  const editor = createEditor({
    path: () => ui().file,
    gitKey: () => `${JSON.stringify(repo()?.git?.summary ?? null)}:${saves()}`,
    byPi,
    onSaved: () => {
      setSaves(saves() + 1);
      void refreshRepo(root);
    },
  });

  // Files and git now and then, and whenever Pi has finished with a tool here.
  let soon: ReturnType<typeof setTimeout> | undefined;
  const refreshSoon = () => {
    clearTimeout(soon);
    soon = setTimeout(() => void refreshRepo(root).catch(() => {}), 250);
  };
  onMount(() => {
    void refreshRepo(root).catch(() => {});
    const every = setInterval(() => {
      if (document.visibilityState !== 'visible') return;
      void refreshRepo(root).catch(() => {});
    }, 5000);
    // Open files that changed on disk show what's there now, unless yours had changes.
    const disk = setInterval(() => {
      if (document.visibilityState !== 'visible') return;
      void checkDisk(ui().tabs)
        .then((fresh) => {
          for (const [path, text] of fresh) editor.replace(path, text);
          if (fresh.size > 0) refreshSoon();
        })
        .catch(() => {});
    }, 1500);
    const off = onEvent((event) => {
      if (event.event !== 'session' || event.change.change !== 'entry') return;
      const entry = event.change.entry;
      const cwd = live[event.id]?.state.cwd ?? '';
      if (entry.kind === 'tool' && entry.status !== 'running' && cwd.startsWith(root)) refreshSoon();
    });
    const onKey = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      if (ctrl && e.key.toLowerCase() === 'p') setQuick(e.shiftKey ? '>' : '');
      else if (ctrl && e.shiftKey && (e.key === '`' || e.key === '~')) void newTerminal();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      clearInterval(every);
      clearInterval(disk);
      clearTimeout(soon);
      off();
      window.removeEventListener('keydown', onKey);
    });
  });

  const relative = (path: string) => (path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path);
  const absolute = (path: string) => (path.startsWith('/') ? path : `${root}/${path.replace(/^\.\//, '')}`);
  const open = (path: string) => {
    const full = absolute(path);
    openFile(props.id, full);
    setRecent([relative(full), ...recent().filter((p) => p !== relative(full))].slice(0, 8));
  };
  const closeOne = (path: string) => {
    closeFile(props.id, path);
    editor.drop(path);
    forgetBuffer(path);
  };
  const newTerminal = async () => {
    const terminal = await openTerminal(root);
    setUi(props.id, { terminal: terminal.id, termShut: false });
  };

  const commands = (): QuickItem[] => [
    { icon: '>', title: 'Save file', keys: 'Ctrl S', run: () => editor.save() },
    {
      icon: '>',
      title: 'Close file',
      run: () => {
        const file = ui().file;
        if (file && !buffers[file]?.dirty) closeOne(file);
      },
    },
    {
      icon: '>',
      title: 'New terminal',
      detail: 'in this folder',
      keys: 'Ctrl Shift `',
      run: () => void newTerminal(),
    },
    {
      icon: '>',
      title: ui().termShut ? 'Show the terminals' : 'Hide the terminals',
      run: () => setUi(props.id, 'termShut', !ui().termShut),
    },
    {
      icon: '>',
      title: 'New session',
      detail: 'a conversation of its own',
      run: () => showSession(props.id, 'new'),
    },
  ];
  const sessionItems = (): QuickItem[] =>
    here().map((s) => ({
      icon: '#',
      tone: s.state.ask ? 'blue' : s.state.busy ? 'amber' : undefined,
      title: s.state.title || 'New session',
      detail: s.state.ask ? 'Needs you' : s.state.busy ? 'Working' : undefined,
      run: () => showSession(props.id, s.state.file ? { file: s.state.file } : { id: s.id }),
    }));

  return (
    <div class="page space-page">
      <Tree view={view()} writing={writing()} onOpen={open} />
      <main class="space-main">
        <div class="editor-tabs">
          <div class="editor-tablist" role="tablist" aria-label="Open files">
            <For each={ui().tabs}>
              {(path) => {
                const name = () => path.split('/').pop() ?? path;
                const on = () => ui().file === path;
                return (
                  <div class="editor-tab" data-current={on()}>
                    <button
                      type="button"
                      role="tab"
                      class="editor-tab-main"
                      aria-selected={on()}
                      title={relative(path)}
                      ref={(el) => {
                        if (on())
                          queueMicrotask(() => el.scrollIntoView({ block: 'nearest', inline: 'nearest' }));
                      }}
                      onClick={() => setUi(props.id, 'file', path)}
                    >
                      {name()}
                    </button>
                    <Show
                      when={!buffers[path]?.dirty}
                      fallback={
                        <span
                          class="editor-tab-dirty"
                          role="img"
                          title="Not saved (Ctrl S saves)"
                          aria-label="Not saved"
                        />
                      }
                    >
                      <button
                        type="button"
                        class="editor-tab-close"
                        aria-label={`Close ${name()}`}
                        onClick={() => closeOne(path)}
                      >
                        <Icon name="close" size={9} />
                      </button>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
          <For each={integrations(view())}>
            {(it) => (
              <>
                <span class="integration" data-state={it.state} title={it.detail}>
                  {it.name}
                </span>
                <For each={it.actions}>
                  {(action) => (
                    <button type="button" class="button button-small" onClick={action.run}>
                      <Icon name="play" size={10} />
                      {action.label}
                    </button>
                  )}
                </For>
              </>
            )}
          </For>
          <button
            type="button"
            class="icon-button icon-button-small"
            aria-label="Go to a file"
            title="Go to a file, run a command (>), a symbol (@), a line (:) or a session (#) · Ctrl P"
            onClick={() => setQuick('')}
          >
            <Icon name="search" size={13} />
          </button>
        </div>
        {editor.view}
        <Terminals space={props.id} dir={root} />
      </main>
      <Sessions view={view()} onOpenFile={open} />
      <Show when={quick() !== undefined}>
        <QuickOpen
          start={quick()}
          onClose={() => setQuick()}
          sources={{
            files: () => repo()?.files ?? [],
            recent,
            commands,
            sessions: sessionItems,
            openFile: open,
            goToLine: (n) => editor.goToLine(n),
            current: () => (ui().file ? relative(ui().file as string) : undefined),
          }}
        />
      </Show>
    </div>
  );
}
