// A conversation, as it is read: what you said in a block of its own, then
// the agent under its name. What the agent thought and what it ran are one
// line each, which opens. A line that is shut has nothing built inside it,
// long output is cut until you ask for all of it, and only the latest part
// of a long conversation is drawn. That is what keeps a long one light.

import { createEffect, createMemo, createSignal, For, type JSX, Match, on, Show, Switch } from 'solid-js';
import { copy } from '../../components/code';
import { Markdown } from '../../components/Markdown';
import type { Entry } from '../../protocol/Entry';
import type { Tool } from '../../protocol/Tool';

/**
 * How somewhere that is about one thing, such as a pull request's tab,
 * changes what is written here and how it reads.
 */
export interface Dress {
  /** What goes to the agent when you write: your words, with what you are on in front. */
  wrap: (text: string) => string;
  /** What you wrote, as shown: your own words, and where you asked. Hidden when it shows somewhere else. */
  you: (text: string) => { text: string; where?: { label: string; go?: () => void }; hidden: boolean };
  /** What the agent wrote, as shown. */
  agent: (text: string) => string;
}

/** How many of a conversation's entries are drawn. The ones before them are one click away. */
const DRAWN = 120;
/** Output longer than this many lines shows its start and its end until you ask for all of it. */
const START = 30;
const END = 10;

/** The conversation, kept at its end while the agent writes unless you scroll up. */
export function Log(props: {
  entries: Entry[];
  /** The agent's name, written where its part of a turn starts. */
  agent: string;
  cwd: string;
  onOpenFile: (path: string) => void;
  dress?: Dress;
  top?: JSX.Element;
  empty?: string;
}) {
  let box!: HTMLDivElement;
  let following = true;
  // A turn that shows somewhere else is left out here: what you wrote, and all the agent did after it.
  const hidden = createMemo(() => {
    let off = false;
    return props.entries.map((entry) => {
      if (entry.kind === 'you') off = props.dress?.you(entry.text).hidden ?? false;
      return off;
    });
  });
  const you = (text: string) => props.dress?.you(text) ?? { text, hidden: false };

  // Only the latest entries are drawn. Another conversation starts from its end again.
  const [drawn, setDrawn] = createSignal(DRAWN);
  createEffect(
    on(
      () => props.entries,
      () => setDrawn(DRAWN),
      { defer: true },
    ),
  );
  const first = () => Math.max(0, props.entries.length - drawn());
  const shown = (at: number) => at >= first() && !hidden()[at];
  /** The agent's name is written before the first thing it does after you, or after what is not drawn. */
  const leads = (at: number) => {
    if (props.entries[at]?.kind === 'you') return false;
    for (let before = at - 1; before >= first(); before--) {
      if (hidden()[before]) continue;
      return props.entries[before].kind === 'you';
    }
    return true;
  };

  createEffect(() => {
    // Read every entry's text, so this runs as the reply grows.
    for (const entry of props.entries) void ('text' in entry ? entry.text : entry.output);
    if (following) requestAnimationFrame(() => box.scrollTo({ top: box.scrollHeight }));
  });
  return (
    <div
      class="chat-log"
      ref={box}
      onScroll={() => {
        following = box.scrollHeight - box.scrollTop - box.clientHeight < 48;
      }}
    >
      <div class="chat-entries">
        {props.top}
        <Show when={!hidden().includes(false)}>
          <p class="dim chat-empty">{props.empty ?? 'Nothing asked yet.'}</p>
        </Show>
        <Show when={first() > 0}>
          <button
            type="button"
            class="quiet chat-earlier"
            onClick={() => {
              // What is drawn above must not move what you are reading.
              const from = box.scrollHeight - box.scrollTop;
              setDrawn(drawn() + DRAWN);
              requestAnimationFrame(() => box.scrollTo({ top: box.scrollHeight - from }));
            }}
          >
            Show {Math.min(DRAWN, first())} earlier of {first()}
          </button>
        </Show>
        <For each={props.entries}>
          {(entry, at) => (
            <Show when={shown(at())}>
              <Show when={leads(at())}>
                <span class="entry-agent">{props.agent}</span>
              </Show>
              <Switch>
                <Match when={entry.kind === 'you' && entry}>
                  {(e) => (
                    <div class="entry-mine">
                      <span class="entry-who">
                        You
                        <Show when={you(e().text).where}>
                          {(where) => (
                            <button type="button" class="entry-where" onClick={() => where().go?.()}>
                              on {where().label}
                            </button>
                          )}
                        </Show>
                      </span>
                      <p class="entry-you">{you(e().text).text}</p>
                    </div>
                  )}
                </Match>
                <Match when={entry.kind === 'text' && entry}>
                  {(e) => <Markdown text={props.dress ? props.dress.agent(e().text) : e().text} />}
                </Match>
                <Match when={entry.kind === 'thinking' && entry}>
                  {(e) => (
                    // An agent that keeps its thinking to itself sends none: there is nothing to open.
                    <Show when={e().text.trim()}>
                      <Fold name="Thought" hint={e().text.trim().split('\n')[0]}>
                        {() => <p class="fold-words">{e().text.trim()}</p>}
                      </Fold>
                    </Show>
                  )}
                </Match>
                <Match when={entry.kind === 'tool' && entry}>
                  {(e) => <ToolRow tool={e()} cwd={props.cwd} onOpenFile={props.onOpenFile} />}
                </Match>
                <Match when={entry.kind === 'notice' && entry}>
                  {(e) => (
                    <p class="entry-notice" data-tone={e().tone}>
                      {e().text}
                    </p>
                  )}
                </Match>
              </Switch>
            </Show>
          )}
        </For>
      </div>
    </div>
  );
}

/**
 * One line that opens. What is inside is only made when it is open, and is
 * thrown away when it shuts: a hundred shut lines hold nothing.
 */
function Fold(props: {
  name: string;
  /** Said in ink after the name: what was run. */
  title?: string;
  /** Said quietly after the name: how what is inside starts. */
  hint?: string;
  /** At the end of the line: a count, or how it went. */
  end?: JSX.Element;
  state?: string;
  /** There is nothing inside. */
  empty?: boolean;
  children: () => JSX.Element;
}) {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="fold" data-state={props.state}>
      <button
        type="button"
        class="fold-head"
        aria-expanded={props.empty ? undefined : open()}
        disabled={props.empty}
        onClick={() => setOpen(!open())}
      >
        <svg
          class="fold-arrow"
          aria-hidden="true"
          width="9"
          height="9"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="3"
        >
          <path d="M9 6l6 6-6 6" />
        </svg>
        <span class="fold-name">{props.name}</span>
        <Show when={props.title}>
          <span class="fold-title">{props.title}</span>
        </Show>
        <Show when={props.hint && !open()}>
          <span class="fold-hint">{props.hint}</span>
        </Show>
        <span class="top-gap" />
        {props.end}
      </button>
      <Show when={open() && !props.empty}>
        <div class="fold-body">{props.children()}</div>
      </Show>
    </div>
  );
}

/** A word that copies something. It says so once it has. */
function CopyWord(props: { text: () => string }) {
  const [said, setSaid] = createSignal('Copy');
  return (
    <button
      type="button"
      class="quiet code-copy"
      onClick={() =>
        void copy(props.text()).then((did) => {
          setSaid(did ? 'Copied' : 'Not copied');
          setTimeout(() => setSaid('Copy'), 1500);
        })
      }
    >
      {said()}
    </button>
  );
}

/** What something printed. Long, it shows its start and its end until you ask for all of it. */
function Output(props: { text: string; name: string; tone?: 'old' | 'new' }) {
  const [all, setAll] = createSignal(false);
  const lines = createMemo(() => props.text.replace(/\n$/, '').split('\n'));
  const cut = () => !all() && lines().length > START + END + 5;
  return (
    <div class="code" data-tone={props.tone}>
      <div class="code-head">
        <span>
          {props.name} · {lines().length} {lines().length === 1 ? 'line' : 'lines'}
        </span>
        <CopyWord text={() => props.text} />
      </div>
      <Show when={cut()} fallback={<pre>{lines().join('\n')}</pre>}>
        <pre>{lines().slice(0, START).join('\n')}</pre>
        <button type="button" class="quiet code-more" onClick={() => setAll(true)}>
          {lines().length - START - END} more lines
        </button>
        <pre>{lines().slice(-END).join('\n')}</pre>
      </Show>
    </div>
  );
}

/** Something the agent did: one line, which opens to what it changed or printed. */
function ToolRow(props: { tool: Tool; cwd: string; onOpenFile: (path: string) => void }) {
  const count = (text: string) => text.split('\n').length;
  /** The file an edit changed, as an absolute path. */
  const file = () => {
    const path = props.tool.diff?.path;
    return path && (path.startsWith('/') ? path : `${props.cwd}/${path}`);
  };
  return (
    <Fold
      name={props.tool.name}
      title={props.tool.title}
      state={props.tool.status}
      empty={!props.tool.diff && !props.tool.output}
      end={
        <>
          <Show when={props.tool.diff}>
            {(diff) => (
              <span class="fold-count">
                <span data-state="ok">+{count(diff().new)}</span>
                <Show when={diff().old}>{(old) => <span data-state="failed"> -{count(old())}</span>}</Show>
              </span>
            )}
          </Show>
          <Show when={props.tool.status !== 'done'}>
            <span class="fold-status">{props.tool.status === 'running' ? 'running' : 'failed'}</span>
          </Show>
        </>
      }
    >
      {() => (
        <>
          <Show when={props.tool.diff}>
            {(diff) => (
              <>
                <Show when={file()}>
                  {(path) => (
                    <button type="button" class="link fold-open" onClick={() => props.onOpenFile(path())}>
                      Open {diff().path}
                    </button>
                  )}
                </Show>
                <Show when={diff().old}>{(old) => <Output text={old()} name="before" tone="old" />}</Show>
                <Output text={diff().new} name={diff().old ? 'after' : 'written'} tone="new" />
              </>
            )}
          </Show>
          <Show when={!props.tool.diff && props.tool.output}>
            <Output text={props.tool.output} name="printed" />
          </Show>
        </>
      )}
    </Fold>
  );
}
