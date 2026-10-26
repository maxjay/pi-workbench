// Home's codespaces: pinned first, then by when you last worked in them,
// each with its branch and changes, what runs in it and its sessions. The
// path tells folders with the same name apart. A row opens to its newest
// sessions.

import { createSignal, For, Show } from 'solid-js';
import { createAction } from '../components/action';
import { Icon } from '../components/Icon';
import { Empty, ErrorNote } from '../components/Note';
import { openFolderDialog } from '../components/OpenFolder';
import type { CodespaceView } from '../protocol/CodespaceView';
import type { SessionSummary } from '../protocol/SessionSummary';
import { codespaceList, codespaces, forget, openFolder, setPinned } from '../state/codespaces';
import { health } from '../state/health';
import { open } from '../state/tabs';
import { standing, when } from './scratch/Nav';
import { showSession } from './space/ui';

const FEW = 6;

/** A path with home as `~`, split so the part telling it from a namesake can stand out. */
function pathParts(view: CodespaceView, all: CodespaceView[], home: string | undefined) {
  const path =
    home && view.space.path.startsWith(`${home}/`)
      ? `~${view.space.path.slice(home.length)}`
      : view.space.path;
  const segments = path.split('/');
  const namesakes = all.filter((other) => other !== view && other.space.name === view.space.name);
  if (namesakes.length === 0) return { head: path, mark: '', tail: '' };
  // The nearest folder above whose name the others don't share there.
  const others = namesakes.map((o) => o.space.path.split('/'));
  for (let i = segments.length - 2; i > 0; i--) {
    const fromEnd = segments.length - i;
    if (others.every((o) => o[o.length - fromEnd] !== segments[i])) {
      return {
        head: `${segments.slice(0, i).join('/')}/`,
        mark: segments[i],
        tail: `/${segments.slice(i + 1).join('/')}`,
      };
    }
  }
  return { head: path, mark: '', tail: '' };
}

function changes(view: CodespaceView) {
  if (!view.exists) return 'Folder not found';
  const git = view.git;
  if (!git) return 'Not a git folder';
  if (git.files === 0) return 'No changes';
  return `${git.files} file${git.files === 1 ? '' : 's'} · +${git.added} −${git.removed}`;
}

export function HomeSpaces() {
  const [query, setQuery] = createSignal('');
  const [all, setAll] = createSignal(false);
  const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set());
  const act = createAction((work: () => Promise<unknown>) => work());
  const home = () => health()?.agentDir.replace(/\/\.pi\/agent$/, '');

  const matching = () => {
    const words = query().toLowerCase().split(/\s+/).filter(Boolean);
    return codespaces().filter((v) =>
      words.every((w) => `${v.space.name} ${v.space.path}`.toLowerCase().includes(w)),
    );
  };
  const shown = () => (all() || query() ? matching() : matching().slice(0, FEW));
  const needYou = () => codespaces().reduce((n, v) => n + v.needYou, 0);
  const summary = () => {
    const n = codespaces().length;
    if (n === 0) return '';
    const waiting = needYou();
    return `${n} folder${n === 1 ? '' : 's'}${waiting ? ` · ${waiting} session${waiting === 1 ? '' : 's'} need${waiting === 1 ? 's' : ''} you` : ''}`;
  };
  const openSpace = (view: CodespaceView, session?: SessionSummary) =>
    act.run(async () => {
      const opened = await openFolder(view.space.path);
      if (session?.file) showSession(opened.space.id, { file: session.file });
      else if (session?.id) showSession(opened.space.id, { id: session.id });
      open(`space:${opened.space.id}`);
    });
  const toggle = (id: string) => {
    const next = new Set(expanded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  };

  return (
    <section class="list" aria-labelledby="spaces">
      <div class="list-head">
        <h2 id="spaces">Codespaces</h2>
        <span class="count">{summary()}</span>
        <span class="list-head-gap" />
        <Show when={codespaces().length > FEW}>
          <label class="search-field search-field-small">
            <Icon name="search" size={13} />
            <span class="visually-hidden">Filter codespaces</span>
            <input
              placeholder="Filter by name or path"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
          </label>
        </Show>
        <button type="button" class="button button-small" onClick={openFolderDialog}>
          <Icon name="plus" size={12} />
          Open folder
        </button>
      </div>
      <ErrorNote error={act.error() ?? codespaceList.error} />
      <Show
        when={codespaces().length > 0}
        fallback={<Empty>No codespaces yet. Open a folder in WSL to work in it with Pi.</Empty>}
      >
        <div class="spaces">
          <Show when={matching().length === 0}>
            <p class="spaces-none">No codespace matches “{query()}”</p>
          </Show>
          <For each={shown()}>
            {(view, i) => {
              const path = () => pathParts(view, codespaces(), home());
              const isOpen = () => expanded().has(view.space.id);
              const divider = () => {
                const before = shown()[i() - 1];
                if (i() === 0)
                  return view.space.pinned
                    ? 'Pinned'
                    : codespaces().some((v) => v.space.pinned)
                      ? 'Recent'
                      : '';
                return before.space.pinned && !view.space.pinned ? 'Recent' : '';
              };
              return (
                <>
                  <Show when={divider()}>
                    <p class="spaces-divider">{divider()}</p>
                  </Show>
                  <div class="space-row" data-missing={!view.exists}>
                    <div class="space-row-main">
                      <button
                        type="button"
                        class="icon-button icon-button-small"
                        aria-expanded={isOpen()}
                        aria-label={
                          isOpen()
                            ? `Hide ${view.space.name}’s sessions`
                            : `Show ${view.space.name}’s sessions`
                        }
                        onClick={() => toggle(view.space.id)}
                      >
                        <span class="chevron chevron-side" data-open={isOpen()}>
                          <Icon name="chevron" size={10} />
                        </span>
                      </button>
                      <button type="button" class="space-row-open" onClick={() => void openSpace(view)}>
                        <span class="space-row-name">
                          <span>{view.space.name}</span>
                          <span class="mono" title={view.space.path}>
                            {path().head}
                            <b>{path().mark}</b>
                            {path().tail}
                          </span>
                        </span>
                        <span class="space-row-git">
                          <span class="mono">{view.git?.branch ?? '—'}</span>
                          <span>{changes(view)}</span>
                        </span>
                        <span class="space-row-run" data-running={view.running.length > 0}>
                          <span class="space-run-dot" aria-hidden="true" />
                          {view.running.length > 0
                            ? `${view.running[0]} running${view.running.length > 1 ? ` · ${view.running.length - 1} more` : ''}`
                            : 'Nothing running'}
                        </span>
                        <span class="space-row-sessions">
                          <span>
                            {view.sessions === 0
                              ? 'No sessions'
                              : `${view.sessions} session${view.sessions === 1 ? '' : 's'}`}
                          </span>
                          <Show when={view.needYou > 0}>
                            <span class="space-need">
                              {' '}
                              · {view.needYou} need{view.needYou === 1 ? 's' : ''} you
                            </span>
                          </Show>
                        </span>
                        <span class="space-row-when">{when(view.space.opened)}</span>
                      </button>
                      <Show
                        when={view.exists}
                        fallback={
                          <button
                            type="button"
                            class="button button-text button-small"
                            onClick={() => void act.run(() => forget(view.space.id))}
                          >
                            Forget
                          </button>
                        }
                      >
                        <button
                          type="button"
                          class="icon-button icon-button-small space-pin"
                          aria-pressed={view.space.pinned}
                          aria-label={
                            view.space.pinned ? `Unpin ${view.space.name}` : `Pin ${view.space.name}`
                          }
                          title={view.space.pinned ? 'Unpin' : 'Pin to the top'}
                          onClick={() => void act.run(() => setPinned(view.space.id, !view.space.pinned))}
                        >
                          <Icon name="pin" size={14} />
                        </button>
                      </Show>
                    </div>
                    <Show when={isOpen()}>
                      <div class="space-row-sessions-list">
                        <Show when={view.latest.length === 0}>
                          <p class="spaces-none">No sessions yet.</p>
                        </Show>
                        <For each={view.latest}>
                          {(s) => (
                            <button
                              type="button"
                              class="space-session"
                              onClick={() => void openSpace(view, s)}
                            >
                              <span class="pad-dot" data-state={standing(s)} />
                              <span class="space-session-title">{s.title}</span>
                              <span class="space-session-note" data-asking={s.asking}>
                                {s.asking ? 'Needs you' : s.busy ? 'Working' : when(s.updated)}
                              </span>
                            </button>
                          )}
                        </For>
                        <button
                          type="button"
                          class="button button-text button-small space-all"
                          onClick={() => void openSpace(view)}
                        >
                          Open {view.space.name}
                          {view.sessions > 0
                            ? ` · all ${view.sessions} session${view.sessions === 1 ? '' : 's'}`
                            : ''}
                        </button>
                      </div>
                    </Show>
                  </div>
                </>
              );
            }}
          </For>
          <Show when={!query() && codespaces().length > FEW}>
            <button type="button" class="spaces-more" onClick={() => setAll(!all())}>
              {all() ? 'Show fewer' : `Show all ${codespaces().length} codespaces`}
            </button>
          </Show>
        </div>
      </Show>
    </section>
  );
}
