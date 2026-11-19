// The agent, beside a workspace: the conversation, and where you write.
// When the agent asks something, its question takes the place of the box
// you write in. The same parts whichever agent it is.

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  type JSX,
  Match,
  on,
  onMount,
  Show,
  Switch,
} from 'solid-js';
import { createAction } from '../../components/action';
import { Markdown } from '../../components/Markdown';
import type { Agent } from '../../protocol/Agent';
import type { Ask } from '../../protocol/Ask';
import type { Entry } from '../../protocol/Entry';
import type { Offer } from '../../protocol/Offer';
import type { Tool } from '../../protocol/Tool';
import {
  agentName,
  answer,
  askOffer,
  defaultAgent,
  lists,
  offers,
  open,
  type Picked,
  pick,
  picks,
  refreshList,
  send,
  setPicked,
  show,
  start,
  stop,
} from '../../state/conversations';

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

export function Chat(props: {
  cwd: string;
  /** The conversation shown, or undefined for a new one. */
  shown: string | undefined;
  onShow: (id: string | undefined) => void;
  /** Opens a file the agent touched. */
  onOpenFile: (path: string) => void;
  /** What a new conversation here tells the agent before anything you write. */
  brief?: string;
  /** No title and no list of conversations: where it is says what it is about. */
  plain?: boolean;
  /** Hides the agent's column, where it can be hidden. */
  onHide?: () => void;
  /** It has a conversation of its own: don't start on the folder's latest one. */
  own?: boolean;
  dress?: Dress;
  /** The agent a new conversation starts with, when where this is keeps it. */
  agent?: Agent;
  onAgent?: (agent: Agent) => void;
  /** Shown before the conversation. */
  top?: JSX.Element;
  /** Shown over the box you write in. */
  above?: JSX.Element;
  /** What it says while nothing was asked. */
  empty?: string;
  /** What the box you write in says while it is empty. */
  hint?: string;
  /** Gets the box you write in, to put the cursor in it. */
  box?: (el: HTMLTextAreaElement) => void;
}) {
  const conversation = () => (props.shown ? open[props.shown] : undefined);
  const [ownAgent, setOwnAgent] = createSignal<Agent>(defaultAgent());
  const agent = () => props.agent ?? ownAgent();
  const setAgent = (next: Agent) => {
    setOwnAgent(next);
    props.onAgent?.(next);
  };
  const [listing, setListing] = createSignal(false);
  const reading = createAction(show);

  onMount(async () => {
    await refreshList(props.cwd).catch(() => {});
    // Something is shown from the start: the latest conversation here, if there is one.
    const latest = lists[props.cwd]?.[0];
    if (!props.shown && latest && !props.own) props.onShow(latest.id);
  });
  createEffect(
    on(
      () => props.shown,
      (id) => id && void reading.run(id),
    ),
  );

  // The agent talked to: this conversation's, or the one picked for a new one.
  const talkingTo = () => conversation()?.state.agent ?? agent();
  createEffect(() => {
    const to = talkingTo();
    if (!offers[to]) void askOffer(to).catch(() => {});
  });
  const picked = (): Picked => {
    const state = conversation()?.state;
    return state ? { model: state.model, effort: state.effort } : picks[agent()];
  };
  const picking = createAction(async (next: Picked) => {
    if (props.shown) return setPicked(props.shown, talkingTo(), next);
    pick(agent(), next);
  });

  const say = createAction(async (text: string) => {
    const id = props.shown;
    const words = props.dress ? props.dress.wrap(text) : text;
    if (id) return send(id, words);
    props.onShow(await start(agent(), props.cwd, words, props.brief));
  });

  return (
    <section class="chat" aria-label="The agent">
      <div class="bar chat-head">
        <span class="chat-agent">{agentName[conversation()?.state.agent ?? agent()]}</span>
        <Show
          when={!props.plain}
          fallback={
            <>
              <span class="top-gap" />
              <Show when={conversation()?.state.busy}>
                <span class="chat-busy">working</span>
              </Show>
            </>
          }
        >
          <button
            type="button"
            class="chat-title"
            aria-expanded={listing()}
            onClick={() => setListing(!listing())}
          >
            {conversation()?.state.title ?? 'New conversation'}
          </button>
        </Show>
        <button type="button" class="strip-tab" onClick={() => props.onShow(undefined)}>
          New
        </button>
        <Show when={props.onHide}>
          {(hide) => (
            <button type="button" class="strip-tab" onClick={() => hide()()}>
              Hide
            </button>
          )}
        </Show>
      </div>
      <Show when={listing()}>
        <div class="chat-list">
          <For each={lists[props.cwd]} fallback={<p class="left-note dim">No conversations here yet.</p>}>
            {(c) => (
              <button
                type="button"
                class="row chat-list-row"
                aria-current={props.shown === c.id ? 'true' : undefined}
                onClick={() => {
                  props.onShow(c.id);
                  setListing(false);
                }}
              >
                <span class="tree-name">{c.title}</span>
                <span class="dim">{agentName[c.agent]}</span>
              </button>
            )}
          </For>
        </div>
      </Show>

      <Log
        entries={conversation()?.entries ?? []}
        cwd={props.cwd}
        onOpenFile={props.onOpenFile}
        dress={props.dress}
        top={props.top}
        empty={props.empty}
      />

      <div class="chat-foot">
        <Show when={reading.error() ?? say.error() ?? picking.error()}>
          {(error) => <p class="note-failed">{error()}</p>}
        </Show>
        <Show when={conversation()?.state.stopped}>{(why) => <p class="note-failed">{why()}</p>}</Show>
        <Show
          when={conversation()?.state.ask}
          fallback={
            <Composer
              above={props.above}
              hint={props.hint}
              box={props.box}
              busy={conversation()?.state.busy ?? false}
              sending={say.busy()}
              agent={props.shown ? undefined : agent()}
              onAgent={setAgent}
              offer={offers[talkingTo()]}
              picked={picked()}
              onPick={(next) => void picking.run(next)}
              onSend={(text) => void say.run(text)}
              onStop={() => props.shown && void stop(props.shown)}
            />
          }
        >
          {(asked) => (
            <AskCard ask={asked()} onAnswer={(a) => props.shown && void answer(props.shown, asked().id, a)} />
          )}
        </Show>
      </div>
    </section>
  );
}

/** The conversation, kept at its end while the agent writes unless you scroll up. */
function Log(props: {
  entries: Entry[];
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
        <For each={props.entries}>
          {(entry, at) => (
            <Switch>
              <Match when={hidden()[at()]}>{null}</Match>
              <Match when={entry.kind === 'you' && entry}>
                {(e) => (
                  <div class="entry">
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
                  <details class="entry-thinking">
                    <summary>Thought</summary>
                    <p>{e().text}</p>
                  </details>
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
          )}
        </For>
      </div>
    </div>
  );
}

/** Something the agent did: one line, which opens to what it changed or printed. */
function ToolRow(props: { tool: Tool; cwd: string; onOpenFile: (path: string) => void }) {
  const lines = (text: string) => text.split('\n').length;
  /** The file an edit changed, as an absolute path. */
  const file = () => {
    const path = props.tool.diff?.path;
    return path && (path.startsWith('/') ? path : `${props.cwd}/${path}`);
  };
  return (
    <details class="tool" data-status={props.tool.status}>
      <summary>
        <span class="tool-name">{props.tool.name}</span>
        <span class="tool-title">{props.tool.title}</span>
        <Show when={props.tool.diff}>
          {(diff) => (
            <span class="tool-count">
              <span data-state="ok">+{lines(diff().new)}</span>
              <Show when={diff().old}>{(old) => <span data-state="failed"> -{lines(old())}</span>}</Show>
            </span>
          )}
        </Show>
        <Show when={props.tool.status !== 'done'}>
          <span class="tool-status">{props.tool.status === 'running' ? 'running' : 'failed'}</span>
        </Show>
      </summary>
      <Show when={props.tool.diff}>
        {(diff) => (
          <div class="tool-body">
            <Show when={file()}>
              {(path) => (
                <button type="button" class="tool-open" onClick={() => props.onOpenFile(path())}>
                  Open {diff().path}
                </button>
              )}
            </Show>
            <Show when={diff().old}>{(old) => <pre class="tool-old">{old()}</pre>}</Show>
            <pre class="tool-new">{diff().new}</pre>
          </div>
        )}
      </Show>
      <Show when={!props.tool.diff && props.tool.output}>
        <pre class="tool-body">{props.tool.output}</pre>
      </Show>
    </details>
  );
}

/** The agent's question, where the box you write in was. */
function AskCard(props: { ask: Ask; onAnswer: (answer: string | null) => void }) {
  const [text, setText] = createSignal('');
  return (
    <form
      class="ask"
      onSubmit={(e) => {
        e.preventDefault();
        if (text().trim()) props.onAnswer(text().trim());
      }}
    >
      <span class="ask-label">The agent asks</span>
      <span>{props.ask.title}</span>
      <Show when={props.ask.message}>{(message) => <span class="ask-message">{message()}</span>}</Show>
      <Show when={props.ask.input}>
        <input
          class="ask-input"
          aria-label="Your answer"
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          ref={(el) => requestAnimationFrame(() => el.focus())}
        />
      </Show>
      <div class="ask-buttons">
        <For each={props.ask.options}>
          {(option, i) => (
            <button
              type="button"
              class="button"
              classList={{ 'button-primary': i() === 0 }}
              onClick={() => props.onAnswer(option)}
            >
              {option}
            </button>
          )}
        </For>
        <Show when={props.ask.input}>
          <button type="submit" class="button button-primary" disabled={!text().trim()}>
            Answer
          </button>
        </Show>
        <Show when={props.ask.input || props.ask.options.length === 0}>
          <button type="button" class="button" onClick={() => props.onAnswer(null)}>
            Dismiss
          </button>
        </Show>
      </div>
    </form>
  );
}

/** Where you write. Enter sends, Shift Enter makes a new line. */
function Composer(props: {
  above?: JSX.Element;
  hint?: string;
  box?: (el: HTMLTextAreaElement) => void;
  busy: boolean;
  sending: boolean;
  /** The agent picked for a new conversation. Undefined once it has started. */
  agent: Agent | undefined;
  onAgent: (agent: Agent) => void;
  /** What the agent talked to offers, once it has said. */
  offer: Offer | undefined;
  picked: Picked;
  onPick: (picked: Picked) => void;
  onSend: (text: string) => void;
  onStop: () => void;
}) {
  const [text, setText] = createSignal('');
  const ready = () => !props.busy && !props.sending && text().trim() !== '';
  const sendIt = () => {
    if (!ready()) return;
    props.onSend(text().trim());
    setText('');
  };
  return (
    <div class="composer">
      {props.above}
      <textarea
        class="composer-text"
        rows={2}
        ref={(el) => props.box?.(el)}
        placeholder={props.hint ?? (props.agent ? `Ask ${agentName[props.agent]}` : 'Write to the agent')}
        aria-label="Write to the agent"
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key !== 'Enter' || e.shiftKey) return;
          e.preventDefault();
          sendIt();
        }}
      />
      <div class="composer-foot">
        <Show when={props.agent}>
          {(picked) => (
            <fieldset class="composer-agents">
              <legend class="hidden-label">Which agent</legend>
              <For each={['pi', 'claude'] as Agent[]}>
                {(each) => (
                  <button
                    type="button"
                    class="strip-tab"
                    aria-pressed={picked() === each}
                    onClick={() => props.onAgent(each)}
                  >
                    {agentName[each]}
                  </button>
                )}
              </For>
            </fieldset>
          )}
        </Show>
        <Picks offer={props.offer} picked={props.picked} disabled={props.busy} onPick={props.onPick} />
        <span class="top-gap" />
        <Show
          when={props.busy}
          fallback={
            <button type="button" class="button" disabled={!ready()} onClick={sendIt}>
              Send
            </button>
          }
        >
          <span class="dim">Working</span>
          <button type="button" class="button" onClick={props.onStop}>
            Stop
          </button>
        </Show>
      </div>
    </div>
  );
}

/** The model and the effort, from what the agent offers. Shown as the agent's own until you pick. */
function Picks(props: {
  offer: Offer | undefined;
  picked: Picked;
  disabled: boolean;
  onPick: (picked: Picked) => void;
}) {
  const models = () => props.offer?.models ?? [];
  const model = () => props.picked.model ?? props.offer?.model ?? null;
  const efforts = () => models().find((m) => m.id === model())?.efforts ?? [];
  const effort = () => props.picked.effort ?? props.offer?.effort ?? null;
  return (
    <Show when={models().length > 0}>
      <select
        class="pick"
        aria-label="Model"
        disabled={props.disabled}
        onChange={(e) => {
          const next = e.currentTarget.value;
          // An effort the new model doesn't take goes back to the agent's own.
          const takes = models().find((m) => m.id === next)?.efforts ?? [];
          const kept =
            props.picked.effort && takes.includes(props.picked.effort) ? props.picked.effort : null;
          props.onPick({ model: next, effort: kept });
        }}
      >
        <Show when={!models().some((m) => m.id === model())}>
          <option selected disabled>
            Model
          </option>
        </Show>
        <For each={models()}>
          {(m) => (
            <option value={m.id} selected={m.id === model()}>
              {m.name}
            </option>
          )}
        </For>
      </select>
      <Show when={efforts().length > 0}>
        <select
          class="pick"
          aria-label="Effort"
          disabled={props.disabled}
          onChange={(e) => props.onPick({ model: props.picked.model, effort: e.currentTarget.value })}
        >
          <Show when={!efforts().includes(effort() ?? '')}>
            <option selected disabled>
              Effort
            </option>
          </Show>
          <For each={efforts()}>
            {(level) => (
              <option value={level} selected={level === effort()}>
                {level}
              </option>
            )}
          </For>
        </select>
      </Show>
    </Show>
  );
}
