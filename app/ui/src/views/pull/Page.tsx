// A pull request's tab: where you read it, talk about it and decide on it
// without going to Azure DevOps. A bar on top: what it is, your vote, and
// what becomes of it. On the left the overview, the checks and the changed
// files. In the middle the one you picked. On the right the agent, in a
// conversation of this pull request's own: it is told the pull request
// before you write anything, works in a separate checkout of its branch,
// and is told what you are on. Nothing said to it is posted.

import { createEffect, createResource, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { createAction } from '../../components/action';
import { shutOnAway } from '../../components/shut';
import type { Place } from '../../protocol/Place';
import type { PullTo } from '../../protocol/PullTo';
import type { Review } from '../../protocol/Review';
import type { Thread } from '../../protocol/Thread';
import {
  addTalk,
  changed,
  checks,
  dropTalks,
  openReview,
  patchTalk,
  say,
  session,
  setPull,
  setTitle,
  settle,
  type Talk,
  talksOf,
  vote,
} from '../../state/review';
import { active, open, type TabId } from '../../state/tabs';
import { workspaceById } from '../../state/workspaces';
import { Chat, type Dress } from '../workspace/Chat';
import {
  code,
  DRAFT_COMMENT,
  DRAFT_REPLY,
  header,
  readReview,
  reviewPrompt,
  shown as shownText,
  threadText,
  withoutFindings,
} from './agent';
import { Checks, outcome } from './Checks';
import { Diff } from './Diff';
import { Overview } from './Overview';
import { type About, hue, hueOf, type Pull, placeName, voteWord } from './parts';
import { Tree } from './Tree';
import { createTalker, why } from './talker';

/** What the middle shows. */
type Shown = { kind: 'overview' } | { kind: 'checks' } | { kind: 'file'; path: string; at?: number };

export function PullPage(props: { tab: TabId; workspace: string; pr: number }) {
  return (
    <Show
      when={workspaceById(props.workspace)}
      keyed
      fallback={
        <p class="pr-pad dim">The folder this pull request was opened from is not on the list any more.</p>
      }
    >
      {(view) => <Page tab={props.tab} workspace={props.workspace} dir={view.workspace.path} pr={props.pr} />}
    </Show>
  );
}

function Page(props: { tab: TabId; workspace: string; dir: string; pr: number }) {
  const { dir, pr } = props;
  const key = `${dir}#${pr}`;
  // Kept in a store and merged into, so a line that didn't change stays the
  // same line on screen while the pull request is asked about again.
  const [data, setData] = createStore<{ review?: Review }>({});
  const [failure, setFailure] = createSignal<string>();
  const load = async () => {
    try {
      const fresh = await openReview(dir, pr);
      setData('review', reconcile(fresh as Review, { merge: true }));
      setTitle(props.tab, fresh.pr.title);
      setFailure();
    } catch (err) {
      setFailure(why(err));
    }
  };
  const [runs, { refetch: recheck }] = createResource(() => checks(dir, pr));
  createEffect(on(changed, () => void load()));
  onMount(() => {
    // Asked again every half minute while its tab is the one shown.
    const timer = setInterval(() => {
      if (document.hidden || active() !== props.tab) return;
      void load();
      void recheck();
    }, 30_000);
    onCleanup(() => clearInterval(timer));
  });

  const [shown, setShown] = createSignal<Shown>({ kind: 'overview' });
  const [about, setAbout] = createSignal<About>();
  const [opened, setOpened] = createStore<Record<string, boolean>>({});

  // The agent: its own folder, what it is told about the pull request, and its one conversation.
  const placing = session(dir, pr);
  const [place] = createResource(() => placing);
  const talker = createTalker(placing);
  const { conversation, setConversation, agent, setAgent, tell } = talker;
  const talkingTo = talker.name;

  const threadOf = (id: number | undefined) =>
    id === undefined ? undefined : data.review?.threads.find((thread) => thread.id === id);
  /** Where lines are, written so it can be read back: the whole path, then the lines. */
  const linesMark = (at: Place) =>
    `lines:${at.file}:${at.from ?? at.line ?? 0}-${at.line ?? 0}${at.old ? ':old' : ''}`;
  const linesContext = (at: Place) =>
    `${at.file}, ${at.line === null ? 'no line' : `lines ${at.from ?? at.line} to ${at.line}`}${at.old ? ' of the old file' : ''}.\n${data.review ? code(data.review, at) : ''}`;
  const threadContext = (thread: Thread) =>
    `${thread.place ? `${linesContext(thread.place)}\n\n` : 'It is about the whole pull request.\n\n'}${threadText(thread)}`;
  /** A talk's subject, for the agent: where it is, and what is there. */
  const subject = (talk: Talk) => {
    const thread = threadOf(talk.thread);
    if (thread) return { where: `thread:${thread.id}`, context: threadContext(thread) };
    if (talk.place) return { where: linesMark(talk.place), context: linesContext(talk.place) };
    return { where: 'pull', context: 'The whole pull request.' };
  };

  const ask: Pull['ask'] = (id, question) => {
    const talk = talksOf(key).find((t) => t.id === id);
    if (!talk) return;
    const about = subject(talk);
    say(key, id, { who: 'you', text: question });
    const answer = say(key, id, { who: 'agent' });
    tell(header(about.where, about.context, true) + question).then(
      (text) =>
        settle(key, id, answer, { text: text || 'The agent ended its turn without saying anything.' }),
      (err) => settle(key, id, answer, { failed: why(err) }),
    );
  };
  const draft: Pull['draft'] = (id) => {
    const talk = talksOf(key).find((t) => t.id === id);
    if (!talk) return;
    const about = subject(talk);
    const between = talk.said
      .filter((s) => s.text)
      .map((s) => `- ${s.who === 'agent' ? 'You' : 'Me'}: ${s.text}`)
      .join('\n');
    const context = between
      ? `${about.context}\n\nWhat you and I said about it, which nobody else sees:\n${between}`
      : about.context;
    patchTalk(key, id, { draft: { text: '', writing: true } });
    tell(header(about.where, context, true) + (talk.thread !== undefined ? DRAFT_REPLY : DRAFT_COMMENT)).then(
      (text) => patchTalk(key, id, { draft: { text: text.trim(), writing: false } }),
      (err) => {
        patchTalk(key, id, { draft: undefined });
        say(key, id, { who: 'agent', failed: why(err) });
      },
    );
  };

  /** The agent's review of the whole pull request: a summary in the chat, and what it found under the lines. */
  const reviewing = createAction(async () => {
    const review = data.review;
    if (!review) return;
    const answer = await tell(`${header('review', reviewPrompt(review), false)}Review this pull request.`);
    // A new review takes the place of what the last one found and you left alone.
    dropTalks(key, (talk) => Boolean(talk.finding) && !talk.resolved && talk.said.length <= 1 && !talk.draft);
    for (const found of readReview(answer).findings) {
      addTalk(key, {
        place: found.place,
        finding: true,
        said: [{ id: crypto.randomUUID(), who: 'agent', at: Date.now(), text: found.text }],
      });
    }
  });

  const showFile = (path: string, at?: number) => setShown({ kind: 'file', path, at });
  /** Goes to something said: its file, or the overview, with it open. */
  const goTo = (card: string, at: Place | null | undefined) => {
    setOpened(card, true);
    if (at && data.review?.files.some((f) => f.path === at.file)) showFile(at.file);
    else setShown({ kind: 'overview' });
  };
  let composer: HTMLTextAreaElement | undefined;
  const pull = (review: Review): Pull => ({
    dir,
    pr,
    key,
    review,
    agent: talkingTo,
    ask,
    draft,
    about,
    setAbout,
    toChat: (at) => {
      setAbout({ kind: 'lines', place: at });
      composer?.focus();
    },
    opened: (card) => Boolean(opened[card]),
    setOpened,
    showFile,
    openTicket: (id) => open(`wi:${props.workspace}:${id}`),
    reload: load,
  });

  /** What the chat is told you are on, as the words on its chip. */
  const aboutLabel = (on: About) => {
    if (on.kind === 'lines') return placeName(on.place);
    if (on.kind === 'run') return on.name;
    const thread = threadOf(on.thread);
    return thread?.place ? `${placeName(thread.place)} · thread` : 'a thread';
  };
  const aboutGo = (on: About) => {
    if (on.kind === 'lines')
      return showFile(on.place.file, on.place.old ? undefined : (on.place.line ?? undefined));
    if (on.kind === 'run') return setShown({ kind: 'checks' });
    goTo(`t${on.thread}`, threadOf(on.thread)?.place);
  };
  const dress: Dress = {
    wrap: (text) => {
      const on = about();
      if (!on) return text;
      if (on.kind === 'lines') return header(linesMark(on.place), linesContext(on.place), false) + text;
      if (on.kind === 'run') return header(`run:${on.run}:${on.name}`, on.context, false) + text;
      const thread = threadOf(on.thread);
      return thread ? header(`thread:${thread.id}`, threadContext(thread), false) + text : text;
    },
    you: (text) => {
      const said = shownText(text);
      const where = said.where ?? '';
      const lines = where.match(/^lines:(.*):(\d+)-(\d+)(:old)?$/);
      const thread = where.match(/^thread:(\d+)$/);
      const run = where.match(/^run:(\d+):(.*)$/);
      let on: About | undefined;
      if (lines) {
        const at = { file: lines[1], from: Number(lines[2]), line: Number(lines[3]), old: Boolean(lines[4]) };
        on = { kind: 'lines', place: { ...at, outdated: false } };
      } else if (thread) on = { kind: 'thread', thread: Number(thread[1]) };
      else if (run) on = { kind: 'run', run: Number(run[1]), name: run[2], context: '' };
      const tagged = on;
      return {
        text: said.text,
        hidden: said.under,
        where: tagged && { label: aboutLabel(tagged), go: () => aboutGo(tagged) },
      };
    },
    // The list of what a review found is for the app. It is not shown, not even while it is being written.
    agent: (text) => withoutFindings(text).replace(/\n*```findings[\s\S]*$/, ''),
  };

  /** What was asked under lines and in threads, for the top of the chat: each goes to where it is. */
  const small = () =>
    talksOf(key)
      .filter((talk) => talk.said.length > 0 && !talk.resolved)
      .flatMap((talk) => {
        const thread = threadOf(talk.thread);
        if (talk.thread !== undefined && !thread) return [];
        const at = thread ? thread.place : talk.place;
        return [
          {
            card: thread ? `t${thread.id}` : `k${talk.id}`,
            place: at,
            label: at ? placeName(at) : 'overview',
            text: talk.said[0]?.text ?? talk.said[0]?.failed ?? 'working',
          },
        ];
      });

  const latest = () => runs.latest?.[0];
  const shownFile = () => {
    const s = shown();
    return s.kind === 'file' ? s : undefined;
  };
  const openGeneral = (review: Review) => review.threads.filter((t) => !t.place && t.unresolved).length;

  return (
    <Show
      when={data.review}
      fallback={
        <p class="pr-pad" classList={{ dim: !failure(), 'note-failed': Boolean(failure()) }}>
          {failure() ?? 'Asking dova'}
        </p>
      }
    >
      {(review) => {
        // The store keeps the review the same object while it is merged into, so this is made once.
        const p = pull(review());
        return (
          <div class="pr" style={{ '--accent': hue(hueOf(`${review().pr.repo}#${review().pr.id}`)) }}>
            <div class="pr-bar">
              <div class="pr-name">
                <h1>{review().pr.title}</h1>
                <span class="pr-meta">
                  <span class="pr-id">#{review().pr.id}</span> ·{' '}
                  <span data-status={stateOf(review())}>{stateOf(review())}</span> ·{' '}
                  {review().mine ? 'you' : review().author} · {review().pr.sourceBranch} into{' '}
                  {review().pr.targetBranch}
                  <Show when={review().merge === 'conflicts'}>
                    <span class="note-failed"> · conflicts with {review().pr.targetBranch}</span>
                  </Show>
                </span>
              </div>
              <span class="top-gap" />
              <Show when={failure() ?? reviewing.error()}>
                {(reason) => <span class="note-failed pr-late">{reason()}</span>}
              </Show>
              <Show when={reviewing.busy()}>
                <span class="pr-working">reviewing</span>
              </Show>
              <button
                type="button"
                class="quiet pr-review"
                title={`${talkingTo()} reviews the whole pull request. What it finds is for you alone.`}
                disabled={reviewing.busy()}
                onClick={() => void reviewing.run()}
              >
                Review
              </button>
              <Decide review={review()} dir={dir} pr={pr} />
            </div>

            <div class="pr-cols">
              <aside class="pr-left" aria-label="Contents">
                <button
                  type="button"
                  class="row pr-part"
                  aria-current={shown().kind === 'overview' ? 'true' : undefined}
                  onClick={() => setShown({ kind: 'overview' })}
                >
                  <span>Overview</span>
                  <span class="pr-part-word" data-tone="people">
                    {openGeneral(review()) ? `${openGeneral(review())} open` : ''}
                  </span>
                </button>
                <button
                  type="button"
                  class="row pr-part"
                  aria-current={shown().kind === 'checks' ? 'true' : undefined}
                  onClick={() => setShown({ kind: 'checks' })}
                >
                  <span>Checks</span>
                  <Show
                    when={latest()}
                    fallback={<span class="pr-part-word">{runs.loading ? '' : 'none'}</span>}
                  >
                    {(run) => (
                      <span class="pr-part-word" data-state={outcome(run().result, run().status).state}>
                        {run().result === 'succeeded' ? '' : outcome(run().result, run().status).words}
                      </span>
                    )}
                  </Show>
                </button>
                <Tree p={p} current={shownFile()?.path} onPick={(path) => showFile(path)} />
              </aside>

              <section class="pr-middle" aria-label="What you are reading">
                <Show when={shown().kind === 'overview'}>
                  <Overview p={p} onChecks={() => setShown({ kind: 'checks' })} />
                </Show>
                <Show when={shown().kind === 'checks'}>
                  <Checks p={p} runs={runs.latest} onChanged={() => void recheck()} />
                </Show>
                <Show when={shownFile()?.path} keyed>
                  {(path) => <Diff p={p} path={path} at={shownFile()?.at} />}
                </Show>
              </section>

              <Show
                when={place.latest}
                fallback={
                  <section class="chat pr-nochat" aria-label="The agent">
                    <p classList={{ dim: !place.error, 'note-failed': Boolean(place.error) }}>
                      {place.error
                        ? why(place.error)
                        : "Getting the pull request's own folder ready for the agent"}
                    </p>
                  </section>
                }
              >
                {(ready) => (
                  <Chat
                    cwd={ready().cwd}
                    brief={ready().brief}
                    plain
                    shown={conversation()}
                    onShow={setConversation}
                    agent={agent()}
                    onAgent={setAgent}
                    dress={dress}
                    box={(el) => {
                      composer = el;
                    }}
                    hint={
                      !about()
                        ? 'Ask about the pull request'
                        : about()?.kind === 'thread'
                          ? 'Ask about this thread'
                          : about()?.kind === 'run'
                            ? 'Ask about this run'
                            : 'Ask about these lines'
                    }
                    empty="It already has this pull request: the description, the tickets, the diff, the threads and the checks. Click a line, a thread or a run, and it is told about that too. Right-click a line to ask about it there."
                    top={
                      <Show when={small().length > 0}>
                        <div class="chat-small">
                          <span class="chat-small-label">Under the lines</span>
                          <For each={small()}>
                            {(one) => (
                              <button type="button" class="row" onClick={() => goTo(one.card, one.place)}>
                                <span class="sq" data-tone="agent" />
                                <span class="chat-small-place">{one.label}</span>
                                <span class="chat-small-text">{one.text}</span>
                              </button>
                            )}
                          </For>
                        </div>
                      </Show>
                    }
                    above={
                      <Show when={about()}>
                        {(on) => (
                          <div class="chat-on">
                            <span>on</span>
                            <button type="button" class="chat-on-what" onClick={() => aboutGo(on())}>
                              {aboutLabel(on())}
                            </button>
                            <button
                              type="button"
                              class="quiet chat-on-drop"
                              aria-label="Ask about the whole pull request instead"
                              onClick={() => setAbout()}
                            >
                              ×
                            </button>
                          </div>
                        )}
                      </Show>
                    }
                    onOpenFile={(path) => {
                      // A file the agent touched is in its own checkout. If the pull request changes it, show its diff.
                      const relative = path.startsWith(`${ready().cwd}/`)
                        ? path.slice(ready().cwd.length + 1)
                        : path;
                      if (review().files.some((f) => f.path === relative)) showFile(relative);
                    }}
                  />
                )}
              </Show>
            </div>
          </div>
        );
      }}
    </Show>
  );
}

/** Where a pull request stands, in a word. */
function stateOf(review: Review) {
  if (review.pr.isDraft && review.pr.status === 'active') return 'draft';
  return review.pr.status;
}

/** One thing a button's list offers. Its square has the colour of what it does. */
interface Choice {
  label: string;
  tone: string;
  /** It can't be undone: it is clicked twice. */
  sure?: boolean;
  run: () => unknown;
}

/** A block of colour that does one thing, with a list of the others behind its arrow. */
function Soft(props: {
  label: string;
  tone: string;
  /** How much of its colour it has: a wash, more of it, or all of it. */
  weight?: 'soft' | 'strong' | 'solid';
  title?: string;
  more: string;
  busy: boolean;
  choices: Choice[];
  onMain: () => void;
}) {
  const [listed, setListed] = createSignal(false);
  const [armed, setArmed] = createSignal<string>();
  let box!: HTMLDivElement;
  shutOnAway(
    listed,
    () => box,
    () => setListed(false),
  );
  createEffect(() => listed() || setArmed());
  return (
    <div class="softs" data-tone={props.tone} data-weight={props.weight ?? 'soft'} ref={box}>
      <button
        type="button"
        class="softs-main"
        data-alone={props.choices.length === 0 || undefined}
        title={props.title}
        disabled={props.busy}
        onClick={props.onMain}
      >
        {props.label}
      </button>
      <Show when={props.choices.length > 0}>
        <button
          type="button"
          class="softs-more"
          aria-label={props.more}
          aria-expanded={listed()}
          disabled={props.busy}
          onClick={() => setListed(!listed())}
        >
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
      </Show>
      <Show when={listed()}>
        <div class="softs-list">
          <For each={props.choices}>
            {(choice) => (
              <button
                type="button"
                class="row"
                onClick={() => {
                  if (choice.sure && armed() !== choice.label) return setArmed(choice.label);
                  setListed(false);
                  void choice.run();
                }}
              >
                <span class="sq" data-tone={choice.tone} />
                <span classList={{ 'note-failed': armed() === choice.label }}>
                  {armed() === choice.label ? 'Sure?' : choice.label}
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** The two decisions: your vote, and what becomes of the pull request. */
function Decide(props: { review: Review; dir: string; pr: number }) {
  const voting = createAction((value: number) => vote(props.dir, props.pr, value));
  const setting = createAction((to: PullTo) => setPull(props.dir, props.pr, to));
  const review = () => props.review;
  const mine = () => review().vote;
  const live = () => review().pr.status === 'active';
  const [sure, setSure] = createSignal(false);

  const voteTone = () =>
    mine() === 0 && review().mine ? 'quiet' : mine() === 0 ? 'ok' : voteWord(mine()).tone;
  const votes = (): Choice[] => [
    { label: 'Approve with suggestions', tone: 'ok', run: () => voting.run(5) },
    { label: 'Wait for the author', tone: 'working', run: () => voting.run(-5) },
    { label: 'Reject', tone: 'failed', run: () => voting.run(-10) },
  ];

  const complete: Choice = {
    label: 'Complete',
    tone: 'accent',
    sure: true,
    run: () => setting.run('complete'),
  };
  const auto = (): Choice =>
    review().autoComplete
      ? { label: 'Cancel auto-complete', tone: 'accent', run: () => setting.run('cancelAutoComplete') }
      : { label: 'Auto-complete', tone: 'accent', run: () => setting.run('autoComplete') };
  const toDraft: Choice = { label: 'Draft', tone: 'faint', run: () => setting.run('draft') };
  const abandon: Choice = { label: 'Abandon', tone: 'failed', sure: true, run: () => setting.run('abandon') };
  /** Only whoever opened it puts it in draft or abandons it. */
  const own = (choices: Choice[]) => (review().mine ? choices : []);

  return (
    <>
      <Show when={voting.error() ?? setting.error()}>
        {(error) => <span class="note-failed pr-late">{error()}</span>}
      </Show>
      <Show when={live()}>
        <Soft
          label={voteWord(mine()).word}
          tone={voteTone()}
          weight={mine() >= 10 ? 'solid' : mine() === 0 ? 'soft' : 'strong'}
          title={mine() === 0 ? undefined : 'Click to take your vote back'}
          more="Other votes"
          busy={voting.busy()}
          choices={votes()}
          onMain={() => void voting.run(mine() === 0 ? 10 : 0)}
        />
      </Show>
      <Show when={live() && !review().pr.isDraft}>
        <Show
          when={review().ready}
          fallback={
            <Soft
              label={review().autoComplete ? 'Auto-complete on' : 'Auto-complete'}
              tone={review().mine ? 'accent' : 'quiet'}
              weight={review().autoComplete ? 'strong' : 'soft'}
              title={
                review().autoComplete
                  ? 'It completes when its policies are met. Click to cancel.'
                  : 'Completes it when its policies are met'
              }
              more="More"
              busy={setting.busy()}
              choices={[complete, ...own([toDraft, abandon])]}
              onMain={() => void setting.run(review().autoComplete ? 'cancelAutoComplete' : 'autoComplete')}
            />
          }
        >
          <Soft
            label={sure() ? 'Sure?' : 'Complete'}
            tone={sure() ? 'failed' : review().mine ? 'accent' : 'quiet'}
            weight={sure() ? 'strong' : review().mine ? 'solid' : 'soft'}
            title={`Squash into ${review().pr.targetBranch}, delete the branch, close the tickets`}
            more="More"
            busy={setting.busy()}
            choices={[auto(), ...own([toDraft, abandon])]}
            onMain={() => {
              // What can't be undone is clicked twice.
              if (!sure()) {
                setSure(true);
                setTimeout(() => setSure(false), 4000);
                return;
              }
              setSure(false);
              void setting.run('complete');
            }}
          />
        </Show>
      </Show>
      <Show when={live() && review().pr.isDraft && review().mine}>
        <Soft
          label="Ready"
          tone="accent"
          title="Takes it out of draft"
          more="More"
          busy={setting.busy()}
          choices={[abandon]}
          onMain={() => void setting.run('ready')}
        />
      </Show>
      <Show when={review().pr.status === 'abandoned' && review().mine}>
        <Soft
          label="Reactivate"
          tone="accent"
          more="More"
          busy={setting.busy()}
          choices={[]}
          onMain={() => void setting.run('reactivate')}
        />
      </Show>
    </>
  );
}
