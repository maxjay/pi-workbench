// One entry in a conversation: what you asked, what Pi said and thought,
// what it ran, and notices.

import { createSignal, For, Match, Show, Switch } from 'solid-js';
import { Markdown } from '../../components/Markdown';
import type { Entry } from '../../protocol/Entry';
import type { Image } from '../../protocol/Image';
import type { Tool } from '../../protocol/Tool';
import { edits, summarize } from './tools';

/** The entry, if it's of this kind, typed as that kind. */
function of<K extends Entry['kind']>(entry: Entry, kind: K) {
  return entry.kind === kind ? (entry as Extract<Entry, { kind: K }>) : undefined;
}

export function EntryView(props: { entry: Entry }) {
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
      <Match when={of(props.entry, 'thinking')}>{(entry) => <Thinking text={entry().text} />}</Match>
      <Match when={of(props.entry, 'tool')}>{(entry) => <ToolCall tool={entry()} />}</Match>
      <Match when={of(props.entry, 'notice')}>
        {(entry) => (
          <p class="notice" data-tone={entry().tone} role={entry().tone === 'error' ? 'alert' : undefined}>
            {entry().text}
          </p>
        )}
      </Match>
    </Switch>
  );
}

function Thinking(props: { text: string }) {
  const [open, setOpen] = createSignal(false);
  return (
    <button type="button" class="thinking" aria-expanded={open()} onClick={() => setOpen(!open())}>
      <span>Thinking</span>
      <Show when={open()}>
        <span class="thinking-text">{props.text}</span>
      </Show>
    </button>
  );
}

const marks = { writing: '…', running: '›', done: '✓', failed: '✗' } as const;

function ToolCall(props: { tool: Tool }) {
  const [open, setOpen] = createSignal(false);
  const summary = () => summarize(props.tool);
  const hasBody = () => props.tool.output !== '' || props.tool.name === 'edit';
  return (
    <div class="tool" data-status={props.tool.status}>
      <button
        type="button"
        class="tool-head"
        aria-expanded={open()}
        disabled={!hasBody()}
        onClick={() => setOpen(!open())}
      >
        <span class="tool-mark" aria-hidden="true">
          {marks[props.tool.status]}
        </span>
        <span class="tool-verb">{summary().verb}</span>
        <span class="tool-target">{summary().target}</span>
        <Show when={summary().added}>
          <span class="tool-added">+{summary().added}</span>
        </Show>
        <Show when={summary().removed}>
          <span class="tool-removed">−{summary().removed}</span>
        </Show>
      </button>
      <Show when={open()}>
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
