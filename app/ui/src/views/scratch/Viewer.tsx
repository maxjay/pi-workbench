// What Pi wrote or changed in this conversation, beside it: a tab for each
// file you opened, new files as they are, edited ones with Pi's changes
// marked in place. Read again each time Pi touches the file.

import { createResource, For, Match, Show, Switch } from 'solid-js';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { FileContent } from '../../protocol/FileContent';
import type { Session } from '../../protocol/Session';
import type { Tool } from '../../protocol/Tool';
import { readFile } from '../../state/sessions';
import { edits } from './tools';

const name = (path: string) => path.split('/').pop() ?? path;

/** A tool's path as wb-agent lists it: relative ones are taken from the session's folder. */
const resolve = (cwd: string, path: string) => (path.startsWith('/') ? path : `${cwd}/${path}`);

/** The finished writes and edits of `path`, in order. */
function changes(session: Session, path: string) {
  return session.entries.filter((entry): entry is Extract<typeof entry, { kind: 'tool' }> & Tool => {
    if (entry.kind !== 'tool' || entry.status !== 'done') return false;
    if (entry.name !== 'write' && entry.name !== 'edit') return false;
    const target = (entry.args as { path?: unknown } | null)?.path;
    return typeof target === 'string' && resolve(session.state.cwd, target) === path;
  });
}

/** New if Pi's first touch wrote it, edited if it changed one that was there. */
const isNew = (session: Session, path: string) => changes(session, path)[0]?.name === 'write';

interface Line {
  n: string;
  text: string;
  kind: 'same' | 'add' | 'del';
}

/**
 * The file's lines, with what Pi's edits since it last wrote the file
 * marked: the new lines where they now stand, the old ones struck out
 * just above them. An edit whose text isn't in the file any more marks
 * nothing.
 */
export function marked(text: string, applied: { oldText: string; newText: string }[]): Line[] {
  const lines = text.endsWith('\n') ? text.slice(0, -1).split('\n') : text.split('\n');
  const added = new Set<number>();
  const removed = new Map<number, string[]>();
  for (const edit of applied) {
    if (!edit.newText) continue;
    const at = text.indexOf(edit.newText);
    if (at < 0) continue;
    const first = text.slice(0, at).split('\n').length - 1;
    const count = edit.newText.replace(/\n$/, '').split('\n').length;
    for (let i = first; i < first + count; i++) added.add(i);
    const old = edit.oldText.replace(/\n$/, '');
    if (old) removed.set(first, [...(removed.get(first) ?? []), ...old.split('\n')]);
  }
  const out: Line[] = [];
  lines.forEach((line, i) => {
    for (const gone of removed.get(i) ?? []) out.push({ n: '', text: gone, kind: 'del' });
    out.push({ n: String(i + 1), text: line, kind: added.has(i) ? 'add' : 'same' });
  });
  return out;
}

const asText = (content: FileContent) => (content.type === 'text' ? content : undefined);
const asImage = (content: FileContent) => (content.type === 'image' ? content : undefined);

export function Viewer(props: {
  session: Session;
  tabs: string[];
  current: string;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [file] = createResource(
    () => ({ path: props.current, version: changes(props.session, props.current).length }),
    ({ path }) => readFile(props.session, path),
  );
  /** The edits to mark: those since Pi last wrote the whole file. */
  const applied = () => {
    const list = changes(props.session, props.current);
    const lastWrite = list.map((t) => t.name).lastIndexOf('write');
    return list.slice(lastWrite + 1).flatMap((t) => edits((t.args ?? {}) as Record<string, unknown>));
  };

  return (
    <aside class="viewer" aria-label="Files from this conversation">
      <div class="viewer-tabs" role="tablist" aria-label="Files">
        <button
          type="button"
          class="icon-button viewer-close"
          aria-label="Close the viewer"
          onClick={props.onClose}
        >
          <Icon name="close" size={12} />
        </button>
        <For each={props.tabs}>
          {(path) => (
            <button
              type="button"
              role="tab"
              class="viewer-tab"
              aria-selected={path === props.current}
              title={path}
              onClick={() => props.onPick(path)}
            >
              <span class="mono">{name(path)}</span>
              <span class="viewer-badge" data-new={isNew(props.session, path)}>
                {isNew(props.session, path) ? 'New' : 'Edited'}
              </span>
            </button>
          )}
        </For>
      </div>
      <div class="viewer-path">
        <span class="mono" title={props.current}>
          {props.current}
        </span>
        <Show when={file.latest && asText(file.latest.content)}>
          {(content) => (
            <span>
              {isNew(props.session, props.current)
                ? `New · ${marked(content().text, []).length} lines`
                : `Edited · ${plural(marked(content().text, applied()).filter((l) => l.kind === 'add').length, 'line')} changed`}
            </span>
          )}
        </Show>
      </div>
      <ErrorNote error={file.error} />
      <Show when={!file.error && file.latest}>
        {(view) => (
          <div class="viewer-body">
            <Switch>
              <Match when={asText(view().content)}>
                {(content) => (
                  <>
                    <div class="code">
                      <For
                        each={marked(content().text, isNew(props.session, props.current) ? [] : applied())}
                      >
                        {(line) => (
                          <div class="code-line" data-kind={line.kind}>
                            <span class="code-number">{line.n}</span>
                            <span class="code-text">{line.text}</span>
                          </div>
                        )}
                      </For>
                    </div>
                    <Show when={content().truncated}>
                      <p class="viewer-empty">Only the start of this file is shown.</p>
                    </Show>
                  </>
                )}
              </Match>
              <Match when={asImage(view().content)}>
                {(image) => (
                  <img
                    class="viewer-image"
                    alt={name(view().path)}
                    src={`data:${image().mimeType};base64,${image().data}`}
                  />
                )}
              </Match>
              <Match when={view().content.type === 'binary'}>
                <p class="viewer-empty">This file isn’t text or an image.</p>
              </Match>
              <Match when={view().content.type === 'missing'}>
                <p class="viewer-empty">This file isn’t there any more.</p>
              </Match>
            </Switch>
          </div>
        )}
      </Show>
    </aside>
  );
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
