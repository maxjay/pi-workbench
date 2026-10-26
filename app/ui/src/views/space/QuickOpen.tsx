// Ctrl P in a codespace, as in an editor: a file by name; `>` a command;
// `:` a line in the open file; `#` a session; `@` a symbol, once a
// language server says what they are.

import { createMemo, createSignal, For, Show } from 'solid-js';

export interface QuickItem {
  icon: string;
  tone?: string;
  title: string;
  detail?: string;
  keys?: string;
  run: () => void;
}

export interface QuickSources {
  files: () => string[];
  recent: () => string[];
  commands: () => QuickItem[];
  sessions: () => QuickItem[];
  openFile: (path: string) => void;
  goToLine: (n: number) => void;
  /** The file open now, relative, for `:`. */
  current: () => string | undefined;
}

type Mode = 'file' | 'command' | 'line' | 'session' | 'symbol';

const modes: { mode: Mode; prefix: string; label: string }[] = [
  { mode: 'file', prefix: '', label: 'files' },
  { mode: 'command', prefix: '>', label: 'commands' },
  { mode: 'symbol', prefix: '@', label: 'symbols' },
  { mode: 'line', prefix: ':', label: 'line' },
  { mode: 'session', prefix: '#', label: 'sessions' },
];

const modeOf = (text: string): Mode =>
  modes.find((m) => m.prefix && text.startsWith(m.prefix))?.mode ?? 'file';

/** How well `query` matches `path`: its letters in order, closer and later scoring higher. */
export function score(path: string, query: string): number {
  if (!query) return 1;
  const hay = path.toLowerCase();
  const name = hay.slice(hay.lastIndexOf('/') + 1);
  const q = query.toLowerCase();
  if (name.startsWith(q)) return 1000 - name.length;
  if (name.includes(q)) return 800 - name.length;
  if (hay.includes(q)) return 600 - hay.length;
  let at = 0;
  let gaps = 0;
  for (const c of q) {
    const found = hay.indexOf(c, at);
    if (found < 0) return 0;
    gaps += found - at;
    at = found + 1;
  }
  return Math.max(1, 400 - gaps - hay.length / 10);
}

export function QuickOpen(props: { sources: QuickSources; start?: string; onClose: () => void }) {
  const [text, setText] = createSignal(props.start ?? '');
  const [at, setAt] = createSignal(0);
  const mode = () => modeOf(text());
  const query = () =>
    text()
      .slice(modes.find((m) => m.mode === mode())?.prefix.length ?? 0)
      .trim();

  const fileItem = (path: string, recent: boolean): QuickItem => {
    const cut = path.lastIndexOf('/');
    return {
      icon: '·',
      title: path.slice(cut + 1),
      detail: (cut > 0 ? path.slice(0, cut) : '') + (recent ? ' · recently open' : ''),
      run: () => props.sources.openFile(path),
    };
  };

  const items = createMemo((): QuickItem[] => {
    const q = query();
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const has = (item: QuickItem) =>
      words.every((w) => `${item.title} ${item.detail ?? ''}`.toLowerCase().includes(w));
    switch (mode()) {
      case 'command':
        return props.sources.commands().filter(has);
      case 'session':
        return props.sources.sessions().filter(has);
      case 'line': {
        const n = Number.parseInt(q, 10);
        const file = props.sources.current();
        if (!file) return [];
        return [
          {
            icon: ':',
            title: Number.isFinite(n) ? `Go to line ${n}` : 'Type a line number',
            detail: file,
            run: () => Number.isFinite(n) && props.sources.goToLine(n),
          },
        ];
      }
      case 'symbol':
        return [];
      default: {
        const files = props.sources.files();
        if (!q) {
          const recent = props.sources.recent().filter((path) => files.includes(path));
          const rest = files.filter((path) => !recent.includes(path)).slice(0, 40);
          return [...recent, ...rest].map((path) => fileItem(path, recent.includes(path)));
        }
        return files
          .map((path) => [path, score(path, q)] as const)
          .filter(([, s]) => s > 0)
          .sort((a, b) => b[1] - a[1])
          .slice(0, 60)
          .map(([path]) => fileItem(path, false));
      }
    }
  });

  const empty = () => {
    if (mode() === 'symbol') return 'Symbols come from a language server, and none is connected here yet.';
    if (mode() === 'line' && !props.sources.current()) return 'Open a file first.';
    if (mode() === 'session') return 'No sessions match.';
    if (mode() === 'command') return 'No commands match.';
    return query() ? `No files match “${query()}”` : 'No files here.';
  };
  const go = (item: QuickItem | undefined) => {
    if (!item) return;
    props.onClose();
    item.run();
  };

  return (
    <>
      <div class="quick-backdrop" aria-hidden="true" onClick={props.onClose} />
      <div class="quick" role="dialog" aria-label="Quick open">
        <div class="quick-head">
          <span class="tag tag-accent">{modes.find((m) => m.mode === mode())?.label}</span>
          <input
            class="mono"
            aria-label="Quick open"
            ref={(el) => queueMicrotask(() => el.focus())}
            value={text()}
            placeholder="Go to a file, or type > @ : #"
            onInput={(e) => {
              setText(e.currentTarget.value);
              setAt(0);
            }}
            onKeyDown={(e) => {
              const list = items();
              if (e.key === 'ArrowDown') setAt(Math.min(at() + 1, list.length - 1));
              else if (e.key === 'ArrowUp') setAt(Math.max(at() - 1, 0));
              else if (e.key === 'Enter') go(list[at()]);
              else if (e.key === 'Escape') props.onClose();
              else return;
              e.preventDefault();
              e.stopPropagation();
            }}
          />
        </div>
        <div class="quick-list" role="listbox">
          <Show when={items().length === 0}>
            <p class="quick-empty">{empty()}</p>
          </Show>
          <For each={items()}>
            {(item, i) => (
              <button
                type="button"
                role="option"
                class="quick-row"
                aria-selected={i() === at()}
                onMouseMove={() => setAt(i())}
                onClick={() => go(item)}
              >
                <span class="quick-icon mono" data-tone={item.tone}>
                  {item.icon}
                </span>
                <span class="quick-title">{item.title}</span>
                <span class="quick-detail">{item.detail}</span>
                <Show when={item.keys}>
                  <span class="quick-keys mono">{item.keys}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
        <div class="quick-modes">
          <For each={modes}>
            {(m) => (
              <button
                type="button"
                data-on={mode() === m.mode}
                onClick={() => {
                  setText(m.prefix);
                  setAt(0);
                }}
              >
                <span class="mono">{m.prefix || 'abc'}</span> {m.label}
              </button>
            )}
          </For>
        </div>
      </div>
    </>
  );
}
