// The Files list: the workspace's folders and files as a tree, with git's
// mark beside a file that changed.

import { For, Show } from 'solid-js';
import type { GitMark } from '../../protocol/GitMark';
import { toggleFolder } from './ui';

interface Node {
  name: string;
  /** Relative to the workspace. */
  path: string;
  /** A folder's children, folders first. Undefined for a file. */
  children?: Node[];
}

/** Turns sorted relative paths into a tree, folders before files. */
export function tree(files: string[]): Node[] {
  const root: Node[] = [];
  const folders = new Map<string, Node[]>([['', root]]);
  for (const file of files) {
    const parts = file.split('/');
    let path = '';
    let into = root;
    for (const [i, name] of parts.entries()) {
      path = path ? `${path}/${name}` : name;
      if (i === parts.length - 1) {
        into.push({ name, path });
        continue;
      }
      let children = folders.get(path);
      if (!children) {
        children = [];
        folders.set(path, children);
        into.push({ name, path, children });
      }
      into = children;
    }
  }
  const sort = (nodes: Node[]) => {
    nodes.sort((a, b) => Number(!a.children) - Number(!b.children) || a.name.localeCompare(b.name));
    for (const node of nodes) if (node.children) sort(node.children);
  };
  sort(root);
  return root;
}

/** The letter git's mark shows as, and the state it means. */
export const marks: Record<GitMark, { letter: string; state: 'working' | 'ok' | 'failed' }> = {
  modified: { letter: 'M', state: 'working' },
  added: { letter: 'A', state: 'ok' },
  untracked: { letter: 'U', state: 'ok' },
  renamed: { letter: 'R', state: 'working' },
  deleted: { letter: 'D', state: 'failed' },
  conflicted: { letter: '!', state: 'failed' },
};

export function Tree(props: {
  id: string;
  files: string[];
  expanded: string[];
  /** Git's mark for each changed file, by relative path. */
  changed: Map<string, GitMark>;
  /** The file shown in the editor, relative to the workspace. */
  current: string | undefined;
  onOpen: (path: string) => void;
}) {
  const Rows = (rows: { nodes: Node[]; depth: number }) => (
    <For each={rows.nodes}>
      {(node) => (
        <Show
          when={node.children}
          fallback={
            <button
              type="button"
              class="row tree-row"
              style={{ 'padding-left': `${20 + rows.depth * 14}px` }}
              aria-current={props.current === node.path ? 'true' : undefined}
              onClick={() => props.onOpen(node.path)}
            >
              <span class="tree-name">{node.name}</span>
              <Show when={props.changed.get(node.path)}>
                {(mark) => (
                  <span class="mark" data-state={marks[mark()].state}>
                    {marks[mark()].letter}
                  </span>
                )}
              </Show>
            </button>
          }
        >
          {(children) => (
            <>
              <button
                type="button"
                class="row tree-row tree-folder"
                style={{ 'padding-left': `${20 + rows.depth * 14}px` }}
                aria-expanded={props.expanded.includes(node.path)}
                onClick={() => toggleFolder(props.id, node.path)}
              >
                <span class="tree-name">{node.name}/</span>
              </button>
              <Show when={props.expanded.includes(node.path)}>
                <Rows nodes={children()} depth={rows.depth + 1} />
              </Show>
            </>
          )}
        </Show>
      )}
    </For>
  );
  return <Rows nodes={tree(props.files)} depth={0} />;
}
