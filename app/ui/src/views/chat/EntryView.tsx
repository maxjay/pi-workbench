// One entry in a conversation: what you asked, what Pi said and thought,
// what it ran, and notices. A file Pi wrote or edited opens in the viewer.

import { createSignal, For, Match, Show, Switch } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import type { Entry } from '../../protocol/Entry';
import type { Image } from '../../protocol/Image';
import type { Shot } from '../../protocol/Shot';
import type { Tool } from '../../protocol/Tool';
import { showPage } from '../../state/browser';
import { edits, summarize } from './tools';

/** The entry, if it's of this kind, typed as that kind. */
function of<K extends Entry['kind']>(entry: Entry, kind: K) {
  return entry.kind === kind ? (entry as Extract<Entry, { kind: K }>) : undefined;
}

export function EntryView(props: {
  entry: Entry;
  cwd: string;
  /** The newest entry of a run still going. */
  last: boolean;
  onOpenFile: (path: string) => void;
  /** Offered on an error that ended the conversation so far. */
  onRetry?: () => void;
}) {
  return (
    <Switch>
      <Match when={of(props.entry, 'user')}>
        {(entry) => (
          <div class="you">
            <span class="you-mark" aria-hidden="true">
              ›
            </span>
            <div class="you-body">
              <p class="you-text">{entry().text}</p>
              <Images images={entry().images} />
            </div>
          </div>
        )}
      </Match>
      <Match when={of(props.entry, 'text')}>{(entry) => <Markdown text={entry().text} />}</Match>
      <Match when={of(props.entry, 'thinking')}>
        {(entry) => <Thinking text={entry().text} going={props.last} />}
      </Match>
      <Match when={of(props.entry, 'tool')}>
        {(entry) => <ToolCall tool={entry()} cwd={props.cwd} onOpenFile={props.onOpenFile} />}
      </Match>
      <Match when={of(props.entry, 'notice')}>
        {(entry) => (
          <div class="notice" data-tone={entry().tone} role={entry().tone === 'error' ? 'alert' : 'status'}>
            <span class="notice-text">{entry().text}</span>
            <Show when={props.onRetry}>
              {(retry) => (
                <button type="button" class="button button-primary notice-action" onClick={() => retry()()}>
                  Try again
                </button>
              )}
            </Show>
          </div>
        )}
      </Match>
    </Switch>
  );
}

function Thinking(props: { text: string; going: boolean }) {
  const [open, setOpen] = createSignal(false);
  return (
    <button type="button" class="thinking" aria-expanded={open()} onClick={() => setOpen(!open())}>
      <span>{props.going ? 'Thinking…' : 'Thought'}</span>
      <Show when={open()}>
        <span class="thinking-text">{props.text}</span>
      </Show>
    </button>
  );
}

const marks = { writing: '…', running: '›', done: '✓', failed: '✗' } as const;

/** The file a finished write or edit changed, as an absolute path. */
function changed(tool: Tool, cwd: string) {
  if (tool.status !== 'done' || (tool.name !== 'write' && tool.name !== 'edit')) return undefined;
  const path = (tool.args as { path?: unknown } | null)?.path;
  if (typeof path !== 'string') return undefined;
  return path.startsWith('/') ? path : `${cwd}/${path}`;
}

function ToolCall(props: { tool: Tool; cwd: string; onOpenFile: (path: string) => void }) {
  const [open, setOpen] = createSignal(false);
  const summary = () => summarize(props.tool);
  const file = () => changed(props.tool, props.cwd);
  const hasBody = () => props.tool.output !== '' || props.tool.name === 'edit';
  return (
    <div class="tool" data-status={props.tool.status} data-file={file() !== undefined}>
      <button
        type="button"
        class="tool-head"
        aria-expanded={file() ? undefined : open()}
        title={file() ? `Open ${file()?.split('/').pop()} in the viewer` : undefined}
        disabled={!file() && !hasBody()}
        onClick={() => {
          const path = file();
          if (path) props.onOpenFile(path);
          else setOpen(!open());
        }}
      >
        <span class="tool-mark" aria-hidden="true">
          {file() ? '✎' : marks[props.tool.status]}
        </span>
        <span class="tool-verb">{summary().verb}</span>
        <span class="tool-target">{summary().target}</span>
        <Show when={summary().added}>
          <span class="tool-added">+{summary().added}</span>
        </Show>
        <Show when={summary().removed}>
          <span class="tool-removed">−{summary().removed}</span>
        </Show>
        <Show when={props.tool.status === 'running'}>
          <span class="tool-meta">running</span>
        </Show>
      </button>
      <Show when={open() && !file()}>
        <div class="tool-body">
          <Show when={props.tool.name === 'edit'}>
            <For each={edits((props.tool.args ?? {}) as Record<string, unknown>)}>
              {(edit) => (
                <pre class="diff">
                  <For each={edit.oldText.split('\n')}>{(line) => <span class="diff-old">-{line}</span>}</For>
                  <For each={edit.newText.split('\n')}>{(line) => <span class="diff-new">+{line}</span>}</For>
                </pre>
              )}
            </For>
          </Show>
          <Show when={props.tool.output}>
            <pre class="tool-output">{props.tool.output}</pre>
          </Show>
        </div>
      </Show>
      {/* Screenshots show without opening the call. */}
      <Images images={props.tool.images} />
      <Show when={props.tool.browser}>{(shot) => <PageCard shot={shot()} />}</Show>
    </div>
  );
}

/** Where a call left Pi in the browser: the page as it looked, and a way
 * into it. Opened, it's a tab of yours, with this conversation beside it. */
function PageCard(props: { shot: Shot }) {
  const open = () => void showPage(props.shot.tab).catch(() => {});
  const where = () => props.shot.url.replace(/^https?:\/\//, '');
  return (
    <div class="page-card">
      <Show when={props.shot.image}>
        {(image) => (
          <button type="button" class="page-card-shot" title="Open this page in a tab" onClick={open}>
            <img alt={props.shot.title} src={`data:${image().mimeType};base64,${image().data}`} />
          </button>
        )}
      </Show>
      <div class="page-card-foot">
        <span class="page-card-where mono" title={props.shot.url}>
          {where()}
        </span>
        <button type="button" class="button page-card-open" onClick={open}>
          Open tab
        </button>
      </div>
    </div>
  );
}

function Images(props: { images: Image[] }) {
  return (
    <For each={props.images}>
      {(image) => <img class="shot" alt="" src={`data:${image.mimeType};base64,${image.data}`} />}
    </For>
  );
}
