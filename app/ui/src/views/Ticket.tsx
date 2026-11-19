// A ticket's tab: an Azure DevOps work item, read and changed here. A bar
// on top with its state and the way to start work on it. In the middle
// what it says, what it belongs to and holds, the work linked to it, and
// its discussion. On the right the agent, told the ticket before you write
// anything, working in the repo's folder.

import { createEffect, createResource, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { createAction } from '../components/action';
import { Markdown } from '../components/Markdown';
import { type Choice, Menu, RULE } from '../components/Menu';
import { openUrl } from '../components/open';
import type { Ticket } from '../protocol/Ticket';
import type { WorkItemRef } from '../protocol/WorkItemRef';
import { converse } from '../state/conversations';
import { setTitle } from '../state/review';
import { active, open, type TabId } from '../state/tabs';
import {
  commentOnTicket,
  keepSession,
  openTicket,
  sessionOf,
  setTicket,
  startTicket,
  ticketSession,
  touched,
} from '../state/tickets';
import { refreshWorkspaces, workspaceById } from '../state/workspaces';
import { when } from './review/parts';
import { Chat } from './workspace/Chat';

export function TicketPage(props: { tab: TabId; workspace: string; id: number }) {
  return (
    <Show
      when={workspaceById(props.workspace)}
      keyed
      fallback={
        <p class="review-pad dim">The folder this ticket was opened from is not on the list any more.</p>
      }
    >
      {(view) => <Page tab={props.tab} workspace={props.workspace} dir={view.workspace.path} id={props.id} />}
    </Show>
  );
}

/** How a state reads: where it stands, as Azure DevOps sorts states. */
function stateTone(ticket: Ticket) {
  const category = ticket.states.find((s) => s.name === ticket.state)?.category;
  if (category === 'InProgress') return 'working';
  if (category === 'Resolved' || category === 'Completed') return 'ok';
  return 'none';
}

function Page(props: { tab: TabId; workspace: string; dir: string; id: number }) {
  const { dir, id } = props;
  const key = `${dir}#${id}`;
  const [ticket, { refetch }] = createResource(
    () => [touched()] as const,
    () => openTicket(dir, id),
  );
  createEffect(() => {
    const title = ticket.latest?.title;
    if (title) setTitle(props.tab, title);
  });
  onMount(() => {
    // Asked again each minute while its tab is the one shown.
    const timer = setInterval(() => document.hidden || active() !== props.tab || void refetch(), 60_000);
    onCleanup(() => clearInterval(timer));
  });

  const setting = createAction((to: { state?: string; mine?: boolean }) => setTicket(dir, id, to));
  const starting = createAction(async () => {
    await startTicket(dir, id);
    refreshWorkspaces();
    open(`ws:${props.workspace}`);
  });

  // The agent's session for this ticket: the repo's folder, and what it is told about the ticket.
  const [agentShown, setAgentShown] = createSignal(true);
  const [place] = createResource(() => ticketSession(dir, id));
  const [conversation, setConversation] = createSignal(sessionOf(key));
  createEffect(on(conversation, (shown) => keepSession(key, shown), { defer: true }));

  // A comment the agent drafted, for you alone until you post it.
  const [text, setText] = createSignal('');
  const [draft, setDraft] = createSignal<{ words?: string; failed?: string }>();
  const posting = createAction(async (words: string) => {
    if (!words.trim()) return;
    await commentOnTicket(dir, id, words.trim());
    setText('');
  });
  const askForComment = async () => {
    const ready = place.latest;
    if (!ready) return setDraft({ failed: "The agent's folder is not ready yet." });
    setDraft({});
    setAgentShown(true);
    const what = text().trim() || 'Draft a comment that says where this ticket stands.';
    try {
      const said = await converse(
        ready.cwd,
        ready.brief,
        conversation(),
        `${what}\n\nAnswer with only the text of the comment.`,
      );
      setConversation(said.id);
      setDraft({ words: await said.answer });
      setText('');
    } catch (err) {
      setDraft({ failed: err instanceof Error ? err.message : String(err) });
    }
  };

  const more = (t: Ticket): Choice[] => [
    t.mine
      ? { label: 'Unassign', run: () => setting.run({ mine: false }) }
      : { label: 'Assign to me', run: () => setting.run({ mine: true }) },
    RULE,
    { label: 'Azure DevOps', run: () => openUrl(t.url) },
  ];
  const openOther = (item: WorkItemRef) => open(`wi:${props.workspace}:${item.id}`);

  return (
    <Show
      when={ticket.latest}
      fallback={
        <p class="review-pad" classList={{ dim: !ticket.error, 'note-failed': Boolean(ticket.error) }}>
          {ticket.error ? String(ticket.error?.message ?? ticket.error) : 'Asking dova'}
        </p>
      }
    >
      {(t) => (
        <div class="review">
          <div class="review-bar">
            <div class="review-name">
              <h1>{t().title}</h1>
              <span class="dim">
                <span class="accent">#{t().id}</span> · {t().type}
                <Show when={t().iteration}>{(sprint) => <> · {sprint().split('\\').pop()}</>}</Show>
                <Show when={t().points}>{(points) => <> · {points()} points</>}</Show> ·{' '}
                {t().mine ? 'yours' : (t().assignedTo ?? 'nobody')}
              </span>
            </div>
            <span class="top-gap" />
            <Show when={setting.error() ?? starting.error()}>
              {(error) => <span class="note-failed review-late">{error()}</span>}
            </Show>
            <Menu
              label={t().state}
              words
              busy={setting.busy()}
              classList={{ [`state-${stateTone(t())}`]: true }}
              choices={t()
                .states.filter((s) => s.name !== t().state)
                .map((s) => ({ label: s.name, run: () => setting.run({ state: s.name }) }))}
            />
            <Show when={t().branches.length === 0 && t().pulls.length === 0}>
              <button
                type="button"
                class="button button-accent"
                title="Makes a branch for it in this repo's folder, links it, and assigns it to you"
                disabled={starting.busy()}
                onClick={() => void starting.run()}
              >
                {starting.busy() ? 'Starting' : 'Start'}
              </button>
            </Show>
            <Menu label="More" choices={more(t())} />
            <Show when={!agentShown()}>
              <button type="button" class="quiet" onClick={() => setAgentShown(true)}>
                Agent
              </button>
            </Show>
          </div>

          <div class="ticket-cols" data-agent={agentShown()}>
            <section class="review-middle" aria-label="The ticket">
              <div class="overview">
                <For each={t().body} fallback={<span class="dim">It has no description.</span>}>
                  {(part) => (
                    <section class="overview-part">
                      <h2 class="pull-label">{part.label}</h2>
                      <div class="ticket-text">
                        <Markdown text={part.text} />
                      </div>
                    </section>
                  )}
                </For>

                <Show when={t().parent || t().children.length > 0}>
                  <section class="overview-part">
                    <h2 class="pull-label">Belongs to, and holds</h2>
                    <Show when={t().parent}>
                      {(parent) => (
                        <button type="button" class="row overview-line" onClick={() => openOther(parent())}>
                          <span class="dim">parent</span>
                          <span class="accent">#{parent().id}</span>
                          <span class="tree-name">{parent().title}</span>
                          <span class="dim">{parent().state}</span>
                        </button>
                      )}
                    </Show>
                    <For each={t().children}>
                      {(child) => (
                        <button type="button" class="row overview-line" onClick={() => openOther(child)}>
                          <span class="dim">child</span>
                          <span class="accent">#{child.id}</span>
                          <span class="tree-name">{child.title}</span>
                          <span class="dim">{child.state}</span>
                        </button>
                      )}
                    </For>
                  </section>
                </Show>

                <Show when={t().pulls.length > 0 || t().branches.length > 0}>
                  <section class="overview-part">
                    <h2 class="pull-label">Work</h2>
                    <For each={t().pulls}>
                      {(pull) => (
                        <button
                          type="button"
                          class="row overview-line"
                          onClick={() => open(`pr:${props.workspace}:${pull}`)}
                        >
                          <span class="dim">pull request</span>
                          <span class="accent">#{pull}</span>
                        </button>
                      )}
                    </For>
                    <For each={t().branches}>
                      {(branch) => (
                        <div class="row overview-line">
                          <span class="dim">branch</span>
                          <span class="tree-name">{branch}</span>
                        </div>
                      )}
                    </For>
                  </section>
                </Show>

                <section class="overview-part">
                  <h2 class="pull-label">Discussion</h2>
                  <Show when={t().comments.length > 0}>
                    <div class="thread" data-open="false">
                      <For each={t().comments}>
                        {(c) => (
                          <div class="said ticket-said">
                            <span class="said-who">
                              {c.author ?? 'Someone'} · {when(c.publishedAt)}
                            </span>
                            <Markdown text={c.text} />
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                  <Show when={draft()}>
                    {(d) => (
                      <div class="note">
                        <span class="note-label">agent draft</span>
                        <Show
                          when={d().words}
                          fallback={
                            <Show when={d().failed} fallback={<span class="note-working">working</span>}>
                              {(why) => <span class="note-failed">{why()}</span>}
                            </Show>
                          }
                        >
                          {(words) => <Markdown text={words()} />}
                        </Show>
                        <div class="note-buttons">
                          <Show when={d().words}>
                            {(words) => (
                              <button
                                type="button"
                                class="quiet note-post"
                                disabled={posting.busy()}
                                onClick={() => void posting.run(words()).then(() => setDraft())}
                              >
                                Post
                              </button>
                            )}
                          </Show>
                          <Show when={d().words || d().failed}>
                            <button type="button" class="quiet" onClick={() => setDraft()}>
                              Discard
                            </button>
                          </Show>
                        </div>
                      </div>
                    )}
                  </Show>
                  <Show when={posting.error()}>{(error) => <p class="note-failed">{error()}</p>}</Show>
                  <form
                    class="thread thread-foot thread-write"
                    data-open="false"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void posting.run(text());
                    }}
                  >
                    <input
                      class="thread-input"
                      placeholder="Comment"
                      aria-label="Write a comment on the ticket"
                      value={text()}
                      onInput={(e) => setText(e.currentTarget.value)}
                    />
                    <Show when={text().trim()}>
                      <button type="submit" class="quiet" disabled={posting.busy()}>
                        Send
                      </button>
                    </Show>
                    <button type="button" class="quiet thread-act" onClick={() => void askForComment()}>
                      Ask
                    </button>
                  </form>
                </section>
              </div>
            </section>

            <Show when={agentShown()}>
              <Show
                when={place.latest}
                fallback={
                  <section class="chat review-nochat" aria-label="The agent">
                    <p classList={{ dim: !place.error, 'note-failed': Boolean(place.error) }}>
                      {place.error
                        ? String((place.error as Error).message ?? place.error)
                        : 'Telling the agent the ticket'}
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
                    onHide={() => setAgentShown(false)}
                    shown={conversation()}
                    onShow={setConversation}
                    onOpenFile={() => open(`ws:${props.workspace}`)}
                  />
                )}
              </Show>
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}
