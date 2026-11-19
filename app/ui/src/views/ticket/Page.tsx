// A ticket's tab. What it has depends on its type, as Azure DevOps has it:
// a bug has steps to reproduce and a severity, a story has acceptance
// criteria and points. On the left its fields, each changed where it is,
// and what it is linked to. In the middle its words, the tickets under
// it, and its discussion: one chat of comments other people see and what
// you and the agent say to each other. On the right the agent, which is
// told the ticket before you write anything.

import { marked } from 'marked';
import { createEffect, createResource, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { createAction } from '../../components/action';
import { Html, Markdown } from '../../components/Markdown';
import { openUrl } from '../../components/open';
import { shutOnAway } from '../../components/shut';
import type { Field } from '../../protocol/Field';
import type { Part } from '../../protocol/Part';
import type { Ticket } from '../../protocol/Ticket';
import { patchTalk, say, setTitle, settle, talksOf, threadTalk } from '../../state/review';
import { active, markTab, open, type TabId } from '../../state/tabs';
import {
  addChild,
  commentOnTicket,
  editTicket,
  keepSession,
  openTicket,
  sessionOf,
  setField,
  setTicket,
  startTicket,
  ticketSession,
  tickOff,
  touched,
  unlinkChild,
} from '../../state/tickets';
import { refreshWorkspaces, workspaceById } from '../../state/workspaces';
import { header, shown as shownText } from '../pull/agent';
import { pullHue, ticketHue } from '../pull/parts';
import { Said } from '../pull/Said';
import { createTalker, type Host, why } from '../pull/talker';
import { Chat, type Dress } from '../workspace/Chat';
import { toMarkdown } from './markdown';

const DRAFT =
  "Write the comment I should post in this ticket's discussion. Use what we have said about it. Answer with only the text of the comment, ready to post, and nothing else.";

export function TicketPage(props: { tab: TabId; workspace: string; id: number }) {
  return (
    <Show
      when={workspaceById(props.workspace)}
      keyed
      fallback={<p class="pr-pad dim">The folder this ticket was opened from is not on the list any more.</p>}
    >
      {(view) => <Page tab={props.tab} workspace={props.workspace} dir={view.workspace.path} id={props.id} />}
    </Show>
  );
}

/** How a state reads, from where Azure DevOps says it stands. */
function stateTone(ticket: Ticket, state: string) {
  const category = ticket.states.find((s) => s.name === state)?.category;
  if (category === 'InProgress') return 'people';
  if (category === 'Resolved') return 'working';
  if (category === 'Completed') return 'ok';
  if (category === 'Removed') return 'failed';
  return 'faint';
}

/** `Tasks`, `User Stories`. */
const many = (kind: string) => (kind.endsWith('y') ? `${kind.slice(0, -1)}ies` : `${kind}s`);

/** An iteration without its project in front: every one of them starts with it. */
const sprint = (path: string) => (path.includes('\\') ? path.slice(path.indexOf('\\') + 1) : path);

function Page(props: { tab: TabId; workspace: string; dir: string; id: number }) {
  const { dir, id } = props;
  const key = `wi:${dir}#${id}`;
  // Kept in a store and merged into, so what you are writing in stays put while the ticket is asked about again.
  const [data, setData] = createStore<{ ticket?: Ticket }>({});
  const [failure, setFailure] = createSignal<string>();
  const load = async () => {
    try {
      const fresh = await openTicket(dir, id);
      setData('ticket', reconcile(fresh as Ticket, { merge: true }));
      setTitle(props.tab, fresh.title);
      setFailure();
    } catch (err) {
      setFailure(why(err));
    }
  };
  createEffect(on(touched, () => void load()));
  onMount(() => {
    // Asked again each minute while its tab is the one shown.
    const timer = setInterval(() => document.hidden || active() !== props.tab || void load(), 60_000);
    onCleanup(() => clearInterval(timer));
  });

  // The agent: it works in the repo's folder, and is told the ticket first.
  const placing = ticketSession(dir, id);
  const [place] = createResource(() => placing);
  const talker = createTalker(placing);
  // The tab has the colour of the ticket's type, and says when the agent is working or waiting on you.
  createEffect(() =>
    markTab(props.tab, {
      hue: data.ticket ? ticketHue(data.ticket.type, data.ticket.color) : undefined,
      standing: talker.standing(),
    }),
  );
  onCleanup(() => markTab(props.tab, { standing: undefined }));
  talker.setConversation(sessionOf(key));
  createEffect(on(talker.conversation, (shown) => keepSession(key, shown), { defer: true }));
  /** The discussion as the agent is told it, when you ask from there. */
  const discussion = () => {
    const said = (data.ticket?.comments ?? []).map((c) => `- ${c.author ?? 'Someone'}: ${c.text}`).join('\n');
    return `The ticket's discussion${said ? `:\n${said}` : ' has nothing in it yet.'}`;
  };
  const host: Host = {
    key,
    agent: talker.name,
    ask: (talk, question) => {
      say(key, talk, { who: 'you', text: question });
      const answer = say(key, talk, { who: 'agent' });
      talker.tell(header('discussion', discussion(), true) + question).then(
        (text) =>
          settle(key, talk, answer, { text: text || 'The agent ended its turn without saying anything.' }),
        (err) => settle(key, talk, answer, { failed: why(err) }),
      );
    },
    draft: (talk) => {
      const between = (talksOf(key).find((t) => t.id === talk)?.said ?? [])
        .filter((s) => s.text)
        .map((s) => `- ${s.who === 'agent' ? 'You' : 'Me'}: ${s.text}`)
        .join('\n');
      const context = between
        ? `${discussion()}\n\nWhat you and I said about it, which nobody else sees:\n${between}`
        : discussion();
      patchTalk(key, talk, { draft: { text: '', writing: true } });
      talker.tell(header('discussion', context, true) + DRAFT).then(
        (text) => patchTalk(key, talk, { draft: { text: text.trim(), writing: false } }),
        (err) => {
          patchTalk(key, talk, { draft: undefined });
          say(key, talk, { who: 'agent', failed: why(err) });
        },
      );
    },
  };
  const dress: Dress = {
    wrap: (text) => text,
    // What was asked in the discussion shows there, not here too.
    you: (text) => {
      const said = shownText(text);
      return { text: said.text, hidden: said.under };
    },
    agent: (text) => text,
  };

  const setting = createAction((to: { state?: string; mine?: boolean }) => setTicket(dir, id, to));
  const starting = createAction(async () => {
    await startTicket(dir, id);
    refreshWorkspaces();
    open(`ws:${props.workspace}`);
  });
  const [states, setStates] = createSignal(false);
  let stateBox!: HTMLDivElement;
  shutOnAway(
    states,
    () => stateBox,
    () => setStates(false),
  );

  // Changing the title and the parts of the text.
  const [draft, setDraft] = createSignal<{
    title: string;
    parts: Record<string, string>;
    /** Each part as it was when you started, to know which you changed. */
    was: Record<string, string>;
  }>();
  /** A part as the Markdown you change it in. */
  const written = (part: Part) =>
    part.source == null ? part.text : part.markdown ? part.source : toMarkdown(part.source);
  const saving = createAction(async () => {
    const [now, ticket] = [draft(), data.ticket];
    if (!now?.title.trim() || !ticket) return;
    // Only a part you changed is written back: the others stay exactly as they are kept.
    const changed = ticket.parts
      .filter((part) => part.field && now.parts[part.field] !== now.was[part.field])
      .map((part) => {
        const text = now.parts[part.field] ?? '';
        return { field: part.field, text, html: marked.parse(text, { async: false, breaks: true }) };
      });
    const title = now.title.trim() === ticket.title ? null : now.title.trim();
    if (title !== null || changed.length > 0) await editTicket(dir, id, title, changed);
    await load();
    setDraft();
  });

  // The tickets under it.
  const [fresh, setFresh] = createSignal('');
  const adding = createAction(async () => {
    const title = fresh().trim();
    if (!title) return;
    await addChild(dir, id, title);
    setFresh('');
  });
  const ticking = createAction((child: number, done: boolean) => tickOff(dir, child, done));
  const unlinking = createAction((child: number) => unlinkChild(dir, id, child));
  const openOther = (other: number) => open(`wi:${props.workspace}:${other}`);

  return (
    <Show
      when={data.ticket}
      fallback={
        <p class="pr-pad" classList={{ dim: !failure(), 'note-failed': Boolean(failure()) }}>
          {failure() ?? 'Asking dova'}
        </p>
      }
    >
      {(t) => (
        <div class="ticket" style={{ '--accent': ticketHue(t().type, t().color) }}>
          <div class="pr-bar">
            <div class="pr-name">
              <h1>{t().title}</h1>
              <span class="pr-meta">
                <span class="pr-id">#{t().id}</span> · <span class="pr-id">{t().type}</span> ·{' '}
                {t().mine ? 'yours' : t().assignedTo ? `${t().assignedTo} has it` : 'nobody has it'}
              </span>
            </div>
            <span class="top-gap" />
            <Show when={failure() ?? setting.error() ?? starting.error()}>
              {(reason) => <span class="note-failed pr-late">{reason()}</span>}
            </Show>
            <Show when={t().branches.length === 0 && t().pulls.length === 0}>
              <button
                type="button"
                class="quiet pr-review"
                title="A branch for it in this folder, linked to it, and the ticket assigned to you"
                disabled={starting.busy()}
                onClick={() => void starting.run()}
              >
                {starting.busy() ? 'Starting' : 'Start'}
              </button>
            </Show>
            <div class="softs" data-tone="accent" ref={stateBox}>
              <button
                type="button"
                class="softs-main softs-state"
                aria-expanded={states()}
                disabled={setting.busy()}
                onClick={() => setStates(!states())}
              >
                {t().state}
                <svg
                  aria-hidden="true"
                  width="11"
                  height="11"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                >
                  <path d="M6 9l6 6 6-6" />
                </svg>
              </button>
              <Show when={states()}>
                <div class="softs-list">
                  <For each={t().states}>
                    {(state) => (
                      <button
                        type="button"
                        class="row"
                        aria-current={state.name === t().state ? 'true' : undefined}
                        onClick={() => {
                          setStates(false);
                          if (state.name !== t().state) void setting.run({ state: state.name });
                        }}
                      >
                        <span class="sq" data-tone={stateTone(t(), state.name)} />
                        {state.name}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </div>

          <div class="pr-cols">
            <aside class="ticket-left" aria-label="Fields and links">
              <For each={t().fields}>
                {(field) => (
                  <FieldRow
                    field={field}
                    mine={t().mine}
                    onSet={(value) => setField(dir, id, field.field, value)}
                    onAssign={(mine) => setTicket(dir, id, { mine })}
                  />
                )}
              </For>

              <div class="files-rule ticket-rule" aria-hidden="true" />
              <Show when={t().parent}>
                {(parent) => (
                  <>
                    <span class="ticket-label">Parent · {parent().type}</span>
                    <button type="button" class="row ticket-link" onClick={() => openOther(parent().id)}>
                      <span style={{ color: ticketHue(parent().type) }}>#{parent().id}</span>
                      <span class="overview-row-name">{parent().title}</span>
                    </button>
                  </>
                )}
              </Show>
              <Show when={t().pulls.length > 0}>
                <span class="ticket-label">{t().pulls.length === 1 ? 'Pull request' : 'Pull requests'}</span>
                <For each={t().pulls}>
                  {(pull) => (
                    <button
                      type="button"
                      class="row ticket-link"
                      onClick={() => open(`pr:${props.workspace}:${pull}`)}
                    >
                      <span style={{ color: pullHue(pull) }}>#{pull}</span>
                    </button>
                  )}
                </For>
              </Show>
              <Show when={t().branches.length > 0}>
                <span class="ticket-label">{t().branches.length === 1 ? 'Branch' : 'Branches'}</span>
                <For each={t().branches}>{(branch) => <span class="ticket-link">{branch}</span>}</For>
              </Show>
              <span class="ticket-label" />
              <button type="button" class="link ticket-link ticket-azure" onClick={() => openUrl(t().url)}>
                Azure DevOps
              </button>
            </aside>

            <section class="overview" aria-label="The ticket">
              <div class="overview-col ticket-col">
                <Show
                  when={draft()}
                  fallback={
                    <div class="ticket-what">
                      <div class="overview-title">
                        <h2>{t().title}</h2>
                        <button
                          type="button"
                          class="quiet"
                          onClick={() => {
                            const parts = Object.fromEntries(
                              t().parts.map((part) => [part.field, written(part)]),
                            );
                            setDraft({ title: t().title, parts, was: parts });
                          }}
                        >
                          Edit
                        </button>
                      </div>
                      <For each={t().parts}>
                        {(part) => (
                          <div class="ticket-part">
                            <span class="overview-label">{part.label}</span>
                            <Show
                              when={(part.shown ?? part.text).trim()}
                              fallback={<span class="dim">Not written.</span>}
                            >
                              <Show
                                when={part.shown != null}
                                fallback={<p class="ticket-plain">{part.text}</p>}
                              >
                                <Show when={part.markdown} fallback={<Html html={part.shown ?? ''} />}>
                                  <Markdown text={part.shown ?? ''} />
                                </Show>
                              </Show>
                            </Show>
                          </div>
                        )}
                      </For>
                    </div>
                  }
                >
                  {(now) => (
                    <div class="overview-what">
                      <input
                        class="overview-title-input"
                        type="text"
                        aria-label="Title"
                        value={now().title}
                        onInput={(e) => setDraft({ ...now(), title: e.currentTarget.value })}
                      />
                      <For each={t().parts.filter((part) => part.field)}>
                        {(part) => (
                          <div class="ticket-part">
                            <span class="overview-label">{part.label}</span>
                            <textarea
                              class="overview-body-input"
                              aria-label={`${part.label}, as Markdown`}
                              rows={Math.min(
                                14,
                                Math.max(3, (now().parts[part.field] ?? '').split('\n').length + 1),
                              )}
                              value={now().parts[part.field] ?? ''}
                              onInput={(e) =>
                                setDraft({
                                  ...now(),
                                  parts: { ...now().parts, [part.field]: e.currentTarget.value },
                                })
                              }
                            />
                          </div>
                        )}
                      </For>
                      <Show when={saving.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
                      <div class="overview-edit-foot">
                        <span class="top-gap" />
                        <button type="button" class="quiet" onClick={() => setDraft()}>
                          Cancel
                        </button>
                        <button
                          type="button"
                          class="soft"
                          data-tone="accent"
                          disabled={saving.busy() || !now().title.trim()}
                          onClick={() => void saving.run()}
                        >
                          Save
                        </button>
                      </div>
                    </div>
                  )}
                </Show>

                <Show when={t().childKind || t().children.length > 0}>
                  <section class="overview-part" aria-label="Under this ticket">
                    <span class="overview-label">
                      {many(t().childKind ?? 'Ticket')} ·{' '}
                      {t().children.length > 0
                        ? `${t().children.filter((child) => child.done).length} of ${t().children.length} done`
                        : 'none yet'}
                    </span>
                    <For each={t().children}>
                      {(child) => (
                        <div class="ticket-child">
                          <button
                            type="button"
                            class="row ticket-tick"
                            aria-label={child.done ? `Not done: ${child.title}` : `Done: ${child.title}`}
                            aria-pressed={child.done}
                            disabled={ticking.busy()}
                            onClick={() => void ticking.run(child.id, !child.done)}
                          >
                            <Show
                              when={child.task}
                              fallback={<span class="sq" data-tone={child.done ? 'ok' : 'people'} />}
                            >
                              <span class="ticket-box" data-done={child.done || undefined}>
                                {child.done ? '✓' : ''}
                              </span>
                            </Show>
                          </button>
                          <button
                            type="button"
                            class="row ticket-child-open"
                            onClick={() => openOther(child.id)}
                          >
                            <span style={{ color: ticketHue(child.type) }}>#{child.id}</span>
                            <span class="ticket-child-name" data-done={child.done || undefined}>
                              {child.title}
                            </span>
                            <span class="dim">
                              {child.task
                                ? child.done
                                  ? ''
                                  : child.state
                                : `${child.type} · ${child.state}`}
                            </span>
                          </button>
                          <button
                            type="button"
                            class="quiet ticket-unlink"
                            aria-label={`Take #${child.id} out from under this ticket`}
                            title="Takes it out from under this ticket. It is not deleted."
                            disabled={unlinking.busy()}
                            onClick={() => void unlinking.run(child.id)}
                          >
                            ×
                          </button>
                        </div>
                      )}
                    </For>
                    <Show when={adding.error() ?? ticking.error() ?? unlinking.error()}>
                      {(error) => <p class="note-failed">{error()}</p>}
                    </Show>
                    <Show when={t().childKind}>
                      {(kind) => (
                        <input
                          class="overview-comment ticket-add"
                          type="text"
                          aria-label={`Add a ${kind().toLowerCase()}`}
                          placeholder={`Add a ${kind().toLowerCase()}`}
                          value={fresh()}
                          disabled={adding.busy()}
                          onInput={(e) => setFresh(e.currentTarget.value)}
                          onKeyDown={(e) => {
                            if (e.key !== 'Enter') return;
                            e.preventDefault();
                            void adding.run();
                          }}
                        />
                      )}
                    </Show>
                  </section>
                </Show>

                <section class="overview-part overview-comments" aria-label="Discussion">
                  <span class="overview-label">Discussion</span>
                  <Said
                    plain
                    p={host}
                    id={0}
                    talk={threadTalk(key, 0)}
                    posted={t().comments}
                    post={(words) => commentOnTicket(dir, id, words)}
                    place={null}
                  />
                </section>
              </div>
            </section>

            <Show
              when={place.latest}
              fallback={
                <section class="chat pr-nochat" aria-label="The agent">
                  <p classList={{ dim: !place.error, 'note-failed': Boolean(place.error) }}>
                    {place.error ? why(place.error) : 'Reading the ticket for the agent'}
                  </p>
                </section>
              }
            >
              {(ready) => (
                <Chat
                  cwd={ready().cwd}
                  brief={ready().brief}
                  plain
                  own
                  shown={talker.conversation()}
                  onShow={talker.setConversation}
                  agent={talker.agent()}
                  onAgent={talker.setAgent}
                  dress={dress}
                  hint="Ask about the ticket"
                  empty="It already has this ticket: its words, its fields, what is above and under it, the discussion, and any pull request linked to it."
                  onOpenFile={() => open(`ws:${props.workspace}`)}
                />
              )}
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}

/** One field of the ticket's form, changed where it is. */
function FieldRow(props: {
  field: Field;
  mine: boolean;
  onSet: (value: string | null) => Promise<unknown>;
  onAssign: (mine: boolean) => Promise<unknown>;
}) {
  const [listed, setListed] = createSignal(false);
  const setting = createAction(async (value: string | null) => {
    setListed(false);
    if ((value ?? '') === (props.field.value ?? '')) return;
    await props.onSet(value);
  });
  const assigning = createAction(async (mine: boolean) => {
    setListed(false);
    await props.onAssign(mine);
  });
  const path = () => props.field.field === 'System.IterationPath';
  const said = (value: string) => (path() ? sprint(value) : value);
  return (
    <>
      <Show
        when={props.field.kind === 'list' || props.field.kind === 'person'}
        fallback={
          <label class="field-row">
            <span class="field-label">{props.field.label}</span>
            <input
              class="field-input"
              type={props.field.kind === 'date' ? 'date' : 'text'}
              inputmode={props.field.kind === 'number' ? 'decimal' : undefined}
              value={props.field.value ?? ''}
              placeholder="none"
              disabled={setting.busy()}
              onChange={(e) => void setting.run(e.currentTarget.value.trim() || null)}
              onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
            />
          </label>
        }
      >
        <button
          type="button"
          class="row field-row"
          aria-expanded={listed()}
          disabled={setting.busy() || assigning.busy()}
          onClick={() => setListed(!listed())}
        >
          <span class="field-label">{props.field.label}</span>
          <span class="field-value" data-none={!props.field.value || undefined}>
            {props.field.value ? said(props.field.value) : props.field.kind === 'person' ? 'Nobody' : 'none'}
          </span>
          <svg
            aria-hidden="true"
            width="9"
            height="9"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="3"
          >
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
        <Show when={listed()}>
          <div class="field-options">
            <Show
              when={props.field.kind === 'person'}
              fallback={
                <>
                  <For each={props.field.options}>
                    {(option) => (
                      <button
                        type="button"
                        class="row"
                        aria-current={option === props.field.value ? 'true' : undefined}
                        onClick={() => void setting.run(option)}
                      >
                        {said(option)}
                      </button>
                    )}
                  </For>
                  <Show when={props.field.value && !path()}>
                    <button type="button" class="row dim" onClick={() => void setting.run(null)}>
                      none
                    </button>
                  </Show>
                </>
              }
            >
              <button
                type="button"
                class="row"
                aria-current={props.mine ? 'true' : undefined}
                onClick={() => void assigning.run(true)}
              >
                Me
              </button>
              <button
                type="button"
                class="row"
                aria-current={!props.field.value ? 'true' : undefined}
                onClick={() => void assigning.run(false)}
              >
                Nobody
              </button>
            </Show>
          </div>
        </Show>
      </Show>
      <Show when={setting.error() ?? assigning.error()}>
        {(error) => <p class="note-failed field-failed">{error()}</p>}
      </Show>
    </>
  );
}
