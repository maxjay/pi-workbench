// What Pi wrote or changed in this conversation, beside it: a tab per
// file, read again each time Pi touches it.

import { createEffect, createResource, createSignal, For, Match, Show, Switch } from 'solid-js';
import { Icon } from '../../components/Icon';
import { ErrorNote } from '../../components/Note';
import type { FileContent } from '../../protocol/FileContent';
import type { Session } from '../../protocol/Session';
import { readFile } from '../../state/sessions';
import { lines } from './tools';

const name = (path: string) => path.split('/').pop() ?? path;

/** A tool's path as wb-agent lists it: relative ones are taken from the session's folder. */
const resolve = (cwd: string, path: string) => (path.startsWith('/') ? path : `${cwd}/${path}`);

/** How often Pi has finished writing or editing `path`: a new count means new contents. */
function touches(session: Session, path: string) {
  return session.entries.filter((entry) => {
    if (entry.kind !== 'tool' || entry.status !== 'done') return false;
    if (entry.name !== 'write' && entry.name !== 'edit') return false;
    const target = (entry.args as { path?: unknown } | null)?.path;
    return typeof target === 'string' && resolve(session.state.cwd, target) === path;
  }).length;
}

const asText = (content: FileContent) => (content.type === 'text' ? content : undefined);
const asImage = (content: FileContent) => (content.type === 'image' ? content : undefined);

export function Viewer(props: { session: Session; onClose: () => void }) {
  const files = () => [...props.session.state.files].reverse();
  const [picked, setPicked] = createSignal<string>();
  const current = () => (picked() && files().includes(picked() as string) ? picked() : files()[0]);
  // A file Pi just touched comes to the front.
  createEffect(() => setPicked(props.session.state.files.at(-1)));

  const [file] = createResource(
    () => {
      const path = current();
      return path ? { path, version: touches(props.session, path) } : undefined;
    },
    ({ path }) => readFile(props.session.id, path),
  );

  return (
    <aside class="viewer" aria-label="Files from this conversation">
      <div class="viewer-tabs" role="tablist" aria-label="Files">
        <For each={files()}>
          {(path) => (
            <button
              type="button"
              role="tab"
              class="viewer-tab"
              aria-selected={path === current()}
              title={path}
              onClick={() => setPicked(path)}
            >
              {name(path)}
            </button>
          )}
        </For>
        <button
          type="button"
          class="icon-button viewer-close"
          aria-label="Close the viewer"
          onClick={props.onClose}
        >
          <Icon name="close" size={12} />
        </button>
      </div>
      <Show
        when={current()}
        fallback={<p class="viewer-empty">Files Pi writes or edits in this conversation show here.</p>}
      >
        <div class="viewer-path mono" title={current()}>
          {current()}
        </div>
        <ErrorNote error={file.error} />
        <Show when={!file.error && file.latest}>
          {(view) => (
            <div class="viewer-body">
              <Switch>
                <Match when={asText(view().content)}>
                  {(content) => (
                    <>
                      <TextView text={content().text} />
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
      </Show>
    </aside>
  );
}

/** Line numbers and text, as two columns, so a long file is two elements, not thousands. */
function TextView(props: { text: string }) {
  const numbers = () => Array.from({ length: Math.max(1, lines(props.text)) }, (_, i) => i + 1).join('\n');
  return (
    <div class="code">
      <pre class="code-numbers" aria-hidden="true">
        {numbers()}
      </pre>
      <pre class="code-text">{props.text}</pre>
    </div>
  );
}
