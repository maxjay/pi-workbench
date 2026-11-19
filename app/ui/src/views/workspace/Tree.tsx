// The Files list: the workspace's folders and files as a tree, with git's
// mark beside a file that changed.

import { For, Show } from 'solid-js';
import type { GitMark } from '../../protocol/GitMark';
import type { Listing } from '../../state/repo';

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
  /** What each folder that was read holds, by path relative to the workspace. */
  folders: Record<string, Listing>;
  expanded: string[];
  /** Git's mark for each changed file, by relative path. */
  changed: Map<string, GitMark>;
  /** The file shown in the editor, relative to the workspace. */
  current: string | undefined;
  onOpen: (path: string) => void;
  /** Opens a folder, or shuts it if it is open. */
  onToggle: (path: string) => void;
}) {
  /** The rows for what one folder holds: its folders, then its files. */
  const Rows = (rows: { path: string; depth: number }) => {
    const inside = (name: string) => (rows.path ? `${rows.path}/${name}` : name);
    const indent = () => ({ 'padding-left': `${20 + rows.depth * 14}px` });
    return (
      <>
        <For each={props.folders[rows.path]?.folders}>
          {(name) => (
            <>
              <button
                type="button"
                class="row tree-row tree-folder"
                style={indent()}
                aria-expanded={props.expanded.includes(inside(name))}
                onClick={() => props.onToggle(inside(name))}
              >
                <span class="tree-name">{name}/</span>
              </button>
              <Show when={props.expanded.includes(inside(name))}>
                <Rows path={inside(name)} depth={rows.depth + 1} />
              </Show>
            </>
          )}
        </For>
        <For each={props.folders[rows.path]?.files}>
          {(name) => (
            <button
              type="button"
              class="row tree-row"
              style={indent()}
              aria-current={props.current === inside(name) ? 'true' : undefined}
              onClick={() => props.onOpen(inside(name))}
            >
              <span class="tree-name">{name}</span>
              <Show when={props.changed.get(inside(name))}>
                {(mark) => (
                  <span class="mark" data-state={marks[mark()].state}>
                    {marks[mark()].letter}
                  </span>
                )}
              </Show>
            </button>
          )}
        </For>
      </>
    );
  };
  return <Rows path="" depth={0} />;
}
