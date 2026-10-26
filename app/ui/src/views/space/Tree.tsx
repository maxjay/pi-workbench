// A codespace's files down the left, as a tree: changed ones marked as git
// sees them, their folders dotted, and a dot where Pi is writing now.
// Filtering lists the files whose path has what you typed. A new file is
// named where the filter goes.

import { createMemo, createSignal, For, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { CodespaceView } from '../../protocol/CodespaceView';
import type { GitMark } from '../../protocol/GitMark';
import { saveFile } from '../../state/editor';
import { refreshRepo, repos } from '../../state/repo';
import { setUi, uiOf } from './ui';

interface Row {
  path: string;
  name: string;
  depth: number;
  dir: boolean;
}

const marks: Record<GitMark, { letter: string; tone: string; label: string }> = {
  modified: { letter: 'M', tone: 'amber', label: 'Changed' },
  added: { letter: 'A', tone: 'accent', label: 'Added' },
  untracked: { letter: 'U', tone: 'accent', label: 'New, not tracked yet' },
  deleted: { letter: 'D', tone: 'red', label: 'Deleted' },
  renamed: { letter: 'R', tone: 'blue', label: 'Renamed' },
  conflicted: { letter: '!', tone: 'red', label: 'Conflict' },
};

/** A file's badge: a letter or two for its kind, as editors draw them. */
const kinds: Record<string, [string, string]> = {
  java: ['J', 'amber'],
  kt: ['K', 'violet'],
  kts: ['K', 'violet'],
  ts: ['TS', 'blue'],
  tsx: ['TS', 'blue'],
  js: ['JS', 'amber'],
  jsx: ['JS', 'amber'],
  rs: ['R', 'red'],
  py: ['P', 'blue'],
  go: ['G', 'blue'],
  md: ['M', 'dim'],
  json: ['{}', 'amber'],
  yml: ['Y', 'red'],
  yaml: ['Y', 'red'],
  toml: ['T', 'dim'],
  css: ['#', 'blue'],
  html: ['<>', 'red'],
  sh: ['$', 'accent'],
};

export function badge(name: string): [string, string] {
  const ext = name.includes('.') ? name.split('.').pop()?.toLowerCase() : '';
  return kinds[ext ?? ''] ?? [name.charAt(0).toUpperCase() || '·', 'dim'];
}

export function Tree(props: {
  view: CodespaceView;
  writing: ReadonlySet<string>;
  onOpen: (path: string) => void;
}) {
  const id = () => props.view.space.id;
  const root = () => props.view.space.path;
  const repo = () => repos[root()];
  const ui = () => uiOf(id());
  const [filter, setFilter] = createSignal('');
  const [naming, setNaming] = createSignal(false);

  const changed = createMemo(() => {
    const map = new Map<string, GitMark>();
    for (const file of repo()?.git?.files ?? []) map.set(file.path, file.mark);
    return map;
  });
  /** Folders with a change somewhere inside. */
  const changedDirs = createMemo(() => {
    const dirs = new Set<string>();
    for (const path of changed().keys()) {
      const parts = path.split('/');
      for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
    }
    return dirs;
  });
  const shown = (dir: string) => {
    const now = ui();
    if (now.expanded.includes(dir)) return true;
    if (now.touched.includes(dir)) return false;
    // Untouched folders open where there are changes, or the open file.
    if (changedDirs().has(dir)) return true;
    return now.tabs.some((tab) => tab.startsWith(`${root()}/${dir}/`));
  };
  const toggle = (dir: string) => {
    const open = shown(dir);
    const now = ui();
    setUi(id(), {
      expanded: open ? now.expanded.filter((d) => d !== dir) : [...now.expanded, dir],
      touched: now.touched.includes(dir) ? now.touched : [...now.touched, dir],
    });
  };
  const collapseAll = () => {
    const dirs = new Set<string>();
    for (const path of repo()?.files ?? []) {
      const parts = path.split('/');
      for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
    }
    setUi(id(), { expanded: [], touched: [...dirs] });
  };

  /** Folders first, then files, at each level; only inside open folders. */
  const rows = createMemo((): Row[] => {
    const files = repo()?.files ?? [];
    const query = filter().trim().toLowerCase();
    if (query)
      return files
        .filter((path) => path.toLowerCase().includes(query))
        .slice(0, 300)
        .map((path) => ({ path, name: path, depth: 0, dir: false }));
    interface Node {
      dirs: Map<string, Node>;
      files: string[];
    }
    const top: Node = { dirs: new Map(), files: [] };
    for (const path of files) {
      const parts = path.split('/');
      let node = top;
      for (const part of parts.slice(0, -1)) {
        let next = node.dirs.get(part);
        if (!next) {
          next = { dirs: new Map(), files: [] };
          node.dirs.set(part, next);
        }
        node = next;
      }
      node.files.push(parts[parts.length - 1]);
    }
    const out: Row[] = [];
    const walk = (node: Node, prefix: string, depth: number) => {
      for (const [name, child] of [...node.dirs].sort(([a], [b]) => a.localeCompare(b))) {
        // A folder holding just one folder is one row: java/shop/checkout.
        let label = name;
        let path = prefix ? `${prefix}/${name}` : name;
        let inner = child;
        while (inner.files.length === 0 && inner.dirs.size === 1) {
          const [next, deeper] = [...inner.dirs][0];
          label = `${label}/${next}`;
          path = `${path}/${next}`;
          inner = deeper;
        }
        out.push({ path, name: label, depth, dir: true });
        if (shown(path)) walk(inner, path, depth + 1);
      }
      for (const name of [...node.files].sort((a, b) => a.localeCompare(b)))
        out.push({ path: prefix ? `${prefix}/${name}` : name, name, depth, dir: false });
    };
    walk(top, '', 0);
    return out;
  });

  const create = createAction(async (name: string) => {
    const path = `${root()}/${name.replace(/^\/+/, '')}`;
    await saveFile(path, '', null);
    await refreshRepo(root());
    props.onOpen(path);
    setNaming(false);
  });

  const summary = () => {
    const git = repo()?.git;
    if (!git) return { branch: '', rest: props.view.exists ? 'Not a git folder' : 'Folder not found' };
    const n = git.summary.files;
    return { branch: git.summary.branch, rest: ` · ${n === 0 ? 'no changes' : `${n} changed`}` };
  };

  return (
    <nav class="space-tree" aria-label="Files">
      <div class="space-tree-head">
        <div class="space-tree-title">
          <span>{props.view.space.name}</span>
          <span title={props.view.space.path}>
            <span class="mono">{summary().branch}</span>
            {summary().rest}
          </span>
        </div>
        <button
          type="button"
          class="icon-button"
          aria-label="New file"
          title="New file"
          onClick={() => setNaming(!naming())}
        >
          <Icon name="file-plus" size={15} />
        </button>
        <button
          type="button"
          class="icon-button"
          aria-label="Collapse all folders"
          title="Collapse all folders"
          onClick={collapseAll}
        >
          <Icon name="collapse" size={15} />
        </button>
      </div>
      <Show
        when={naming()}
        fallback={
          <label class="search-field">
            <Icon name="search" size={13} />
            <span class="visually-hidden">Filter files</span>
            <input
              placeholder="Filter files"
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setFilter('');
              }}
            />
          </label>
        }
      >
        <label class="search-field search-field-active">
          <Icon name="file-plus" size={13} />
          <span class="visually-hidden">New file’s name</span>
          <input
            ref={(el) => queueMicrotask(() => el.focus())}
            placeholder="src/new-file.ts"
            disabled={create.busy()}
            onKeyDown={(e) => {
              const name = e.currentTarget.value.trim();
              if (e.key === 'Enter' && name) void create.run(name);
              if (e.key === 'Escape') setNaming(false);
            }}
          />
        </label>
      </Show>
      <ErrorNote error={create.error()} />
      <div class="space-tree-rows" role="tree">
        <For each={rows()}>
          {(row) => {
            const full = () => `${root()}/${row.path}`;
            const mark = () => changed().get(row.path);
            const open = () => row.dir && shown(row.path);
            return (
              <button
                type="button"
                role="treeitem"
                class="tree-row"
                aria-expanded={row.dir ? open() : undefined}
                aria-current={!row.dir && ui().file === full() ? 'true' : undefined}
                title={row.path}
                onClick={() => (row.dir ? toggle(row.path) : props.onOpen(full()))}
              >
                <For each={Array.from({ length: row.depth })}>{() => <span class="tree-guide" />}</For>
                <span class="tree-chevron" data-open={open()}>
                  <Show when={row.dir}>
                    <Icon name="chevron" size={10} />
                  </Show>
                </span>
                <Show
                  when={row.dir}
                  fallback={
                    <span class="tree-badge mono" data-tone={badge(row.name)[1]}>
                      {badge(row.name)[0]}
                    </span>
                  }
                >
                  <span class="tree-folder">
                    <Icon name="folder" size={16} />
                  </span>
                </Show>
                <span class="tree-name" data-filtered={filter().trim() !== ''}>
                  {row.name}
                </span>
                <Show when={props.writing.has(full())}>
                  <span class="tree-pi" title="Pi is writing this" />
                </Show>
                <Show
                  when={mark()}
                  fallback={
                    <Show when={row.dir && changedDirs().has(row.path)}>
                      <span class="tree-mark" data-tone="amber" title="Has changes">
                        •
                      </span>
                    </Show>
                  }
                >
                  {(m) => (
                    <span class="tree-mark mono" data-tone={marks[m()].tone} title={marks[m()].label}>
                      {marks[m()].letter}
                    </span>
                  )}
                </Show>
              </button>
            );
          }}
        </For>
        <Show when={repo()?.truncated}>
          <p class="tree-note">Only the first 5,000 files are listed.</p>
        </Show>
      </div>
    </nav>
  );
}
