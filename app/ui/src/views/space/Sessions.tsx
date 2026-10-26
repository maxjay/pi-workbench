// Pi in a codespace, on the right: the session you're in, and the others a
// click away in a list over the conversation that closes once you pick
// one. Each session is a conversation of its own, in the codespace's
// folder, drawn with the parts every conversation has.

import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { onEvent } from '../../agent';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { CodespaceView } from '../../protocol/CodespaceView';
import type { Session } from '../../protocol/Session';
import type { SessionSummary } from '../../protocol/SessionSummary';
import { kept, live, sessionsIn, showElsewhere, startIn } from '../../state/sessions';
import { Foot } from '../chat/Foot';
import { Log } from '../chat/Log';
import { standing as dotOf, when } from '../scratch/Nav';
import { type Shown, showSession, uiOf } from './ui';

/** A session in the list, with what its live session says now. */
interface Row {
  key: string;
  id?: string;
  file?: string;
  title: string;
  busy: boolean;
  asking: boolean;
  stopped: boolean;
  done: boolean;
  updated: number;
}

export function Sessions(props: { view: CodespaceView; onOpenFile: (path: string) => void }) {
  const space = () => props.view.space;
  const ui = () => uiOf(space().id);
  const [saved, { refetch }] = createResource(
    () => space().path,
    (dir) => sessionsIn(dir),
  );
  // Sessions here starting, stopping, asking or finishing: ask for the list again.
  let soon: ReturnType<typeof setTimeout> | undefined;
  const off = onEvent((event) => {
    if (event.event !== 'session') return;
    const session = live[event.id];
    const here = !session || session.state.cwd.startsWith(space().path);
    if (here && (event.change.change === 'state' || event.change.change === 'closed')) {
      clearTimeout(soon);
      soon = setTimeout(() => void refetch(), 300);
    }
  });
  onCleanup(() => {
    off();
    clearTimeout(soon);
  });

  const rows = createMemo((): Row[] => {
    const list: SessionSummary[] = saved.latest ?? [];
    return list.map((s) => {
      const now = s.id
        ? live[s.id]
        : Object.values(live).find((l) => l.state.file && l.state.file === s.file);
      return {
        key: s.file ?? s.id ?? s.title,
        id: now?.id ?? s.id ?? undefined,
        file: now?.state.file ?? s.file ?? undefined,
        title: now?.state.title || s.title,
        busy: now ? now.state.busy : s.busy,
        asking: now ? now.state.ask !== null : s.asking,
        stopped: now ? now.state.stopped !== null : s.stopped,
        done: now ? now.state.done : s.done,
        updated: s.updated,
      };
    });
  });

  /** The session a `Shown` names: live when Pi runs it, else as read from its file. */
  const resolve = (shown: Shown | undefined): Session | undefined => {
    if (!shown || shown === 'new') return undefined;
    if ('id' in shown) return live[shown.id];
    return Object.values(live).find((s) => s.state.file === shown.file) ?? kept[shown.file];
  };
  const showing = createAction(async (row: Pick<Row, 'id' | 'file'>) => {
    const session = await showElsewhere(row);
    showSession(space().id, session.state.file ? { file: session.state.file } : { id: session.id });
    setPicking(false);
  });
  const session = () => resolve(ui().shown);

  // Something is always shown: the newest session, or a new one when there's none.
  createEffect(() => {
    if (ui().shown || saved.loading || showing.busy()) return;
    const first = rows().find((r) => !r.done) ?? rows()[0];
    if (first) void showing.run(first);
    else if (!saved.loading && saved.state === 'ready') showSession(space().id, 'new');
  });
  // A session just started has a file once Pi says so: name it by that, which outlasts its Pi.
  createEffect(() => {
    const shown = ui().shown;
    if (shown && shown !== 'new' && 'id' in shown) {
      const file = live[shown.id]?.state.file;
      if (file) showSession(space().id, { file });
    }
  });

  const [picking, setPicking] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const matching = () => {
    const words = query().toLowerCase().split(/\s+/).filter(Boolean);
    return rows().filter((r) => words.every((w) => r.title.toLowerCase().includes(w)));
  };
  const current = () => {
    const s = session();
    if (!s) return undefined;
    return rows().find((r) => (r.file && r.file === s.state.file) || (r.id && r.id === s.id));
  };
  const othersWaiting = () => rows().filter((r) => r.asking && r !== current()).length;
  const meta = () => {
    const s = session();
    if (!s) return ui().shown === 'new' ? 'A conversation of its own, in this folder' : '';
    if (s.state.ask) return 'Waiting for you';
    if (s.state.busy) return 'Working';
    if (s.state.stopped) return 'Pi stopped';
    const row = current();
    return row ? `Idle · ${when(row.updated)}` : 'Idle';
  };

  return (
    <aside class="space-sessions" aria-label="Pi in this codespace">
      <div class="session-switch">
        <button
          type="button"
          class="session-switch-main"
          aria-haspopup="dialog"
          aria-expanded={picking()}
          title="Switch session"
          data-open={picking()}
          onClick={() => {
            setQuery('');
            setPicking(!picking());
          }}
        >
          <span
            class="pad-dot"
            data-state={
              session()
                ? dotOf(current() ?? { busy: false, asking: false, stopped: false, done: false })
                : 'idle'
            }
          />
          <span class="session-switch-title">
            <span>{session()?.state.title || (ui().shown === 'new' ? 'New session' : 'Sessions')}</span>
            <span>{meta()}</span>
          </span>
          <Show when={othersWaiting() > 0}>
            <span class="session-waiting" title="Other sessions waiting on you">
              {othersWaiting()} need{othersWaiting() === 1 ? 's' : ''} you
            </span>
          </Show>
          <span class="chevron" data-open={picking()}>
            <Icon name="chevron" size={10} />
          </span>
        </button>
        <button
          type="button"
          class="icon-button"
          aria-label="New session"
          title="New session: a conversation of its own"
          onClick={() => {
            setPicking(false);
            showSession(space().id, 'new');
          }}
        >
          <Icon name="plus" size={14} />
        </button>
        <Show when={picking()}>
          <div class="session-catcher" aria-hidden="true" onClick={() => setPicking(false)} />
          <div
            class="session-menu"
            role="dialog"
            aria-label="Sessions"
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.stopPropagation();
                setPicking(false);
              }
            }}
          >
            <label class="search-field">
              <Icon name="search" size={13} />
              <span class="visually-hidden">Search sessions</span>
              <input
                ref={(el) => queueMicrotask(() => el.focus())}
                placeholder={rows().length ? `Search ${rows().length} sessions` : 'No sessions yet'}
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  const first = matching()[0];
                  if (e.key === 'Enter' && first) void showing.run(first);
                }}
              />
            </label>
            <ul class="session-list" aria-label="Sessions">
              <Show when={matching().length === 0}>
                <li class="session-none">
                  {rows().length ? `No sessions match “${query()}”` : 'No sessions here yet.'}
                </li>
              </Show>
              <For each={matching()}>
                {(row) => (
                  <li>
                    <button
                      type="button"
                      class="session-row"
                      aria-current={row === current() ? 'true' : undefined}
                      onClick={() => void showing.run(row)}
                    >
                      <span class="pad-dot" data-state={dotOf(row)} />
                      <span class="session-row-title">{row.title}</span>
                      <Show when={row.asking}>
                        <span class="session-row-asking">Needs you</span>
                      </Show>
                      <span class="session-row-when">{row.busy ? 'now' : when(row.updated)}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Show>
      </div>
      <ErrorNote error={showing.error() ?? saved.error} />
      <Show when={session()} keyed fallback={<StartBox view={props.view} onStarted={() => void refetch()} />}>
        {(s) => (
          <>
            <Log session={s} onOpenFile={props.onOpenFile} />
            <Foot session={s} />
          </>
        )}
      </Show>
    </aside>
  );
}

/** A session not started yet: what you write starts it, in the codespace's folder. */
function StartBox(props: { view: CodespaceView; onStarted: () => void }) {
  const [text, setText] = createSignal('');
  const start = createAction(async () => {
    const message = text().trim();
    if (!message) return;
    const session = await startIn(props.view.space.path, message);
    setText('');
    showSession(props.view.space.id, session.state.file ? { file: session.state.file } : { id: session.id });
    props.onStarted();
  });
  return (
    <>
      <div class="log">
        <div class="log-content">
          <p class="log-empty">
            {props.view.sessions === 0
              ? 'No sessions here yet. Say what you want to work on; each session is a conversation of its own.'
              : 'A new session. Say what you want to work on.'}
          </p>
        </div>
      </div>
      <div class="conversation-foot">
        <div class="composer">
          <div class="composer-box">
            <textarea
              class="composer-input composer-input-plain"
              rows={2}
              aria-label="What do you want to work on?"
              placeholder="What do you want to work on?"
              value={text()}
              disabled={start.busy()}
              onInput={(e) => setText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  void start.run();
                }
              }}
            />
            <div class="composer-bar">
              <span class="composer-hint">Pi works in {props.view.space.name}, with its terminals</span>
              <button
                type="button"
                class="send"
                aria-label="Start"
                disabled={start.busy() || !text().trim()}
                onClick={() => void start.run()}
              >
                <Icon name="send" size={16} />
              </button>
            </div>
          </div>
          <ErrorNote error={start.error()} />
        </div>
        <div class="status-line mono">
          <span title={props.view.space.path}>{props.view.space.path}</span>
        </div>
      </div>
    </>
  );
}
