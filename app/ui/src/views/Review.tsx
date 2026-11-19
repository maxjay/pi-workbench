// A pull request's tab: where you review it without going to Azure DevOps.
// A bar on top that stays put: what it is, its check, the votes, and the
// two decisions. On the left the overview, the checks and the changed
// files. In the middle the one you picked. On the right the agent, in a
// session of this pull request's own: it is told the pull request before
// you write anything, and works in a separate checkout of its branch.

import { createEffect, createResource, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { createAction } from '../components/action';
import type { PullTo } from '../protocol/PullTo';
import type { Review } from '../protocol/Review';
import { open as conversations, defaultAgent, send, show, start, turnEnd } from '../state/conversations';
import {
  addNote,
  addNotes,
  changed,
  checks,
  dropNotes,
  type Note,
  openReview,
  session,
  setPull,
  setTitle,
  settleNote,
  vote,
} from '../state/review';
import { active, type TabId } from '../state/tabs';
import { workspaceById } from '../state/workspaces';
import { FROM_REVIEW, readReview, reviewPrompt } from './review/agentReview';
import { Checks, outcome } from './review/Checks';
import { FileDiff } from './review/FileDiff';
import { Files } from './review/Files';
import { Overview } from './review/Overview';
import { type Reviewing, voteWords } from './review/parts';
import { Chat } from './workspace/Chat';

/** What the middle shows. */
type Shown = { kind: 'overview' } | { kind: 'checks' } | { kind: 'file'; path: string; at?: number };

export function ReviewPage(props: { tab: TabId; workspace: string; pr: number }) {
  return (
    <Show
      when={workspaceById(props.workspace)}
      keyed
      fallback={
        <p class="review-pad dim">
          The folder this pull request was opened from is not on the list any more.
        </p>
      }
    >
      {(view) => <Page tab={props.tab} dir={view.workspace.path} pr={props.pr} />}
    </Show>
  );
}

function Page(props: { tab: TabId; dir: string; pr: number }) {
  const { dir, pr } = props;
  const pull = `${dir}#${pr}`;
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
      setFailure(err instanceof Error ? err.message : String(err));
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
  const [agentShown, setAgentShown] = createSignal(true);

  // The agent's session: its own folder, and what it is told about the pull request.
  const [place] = createResource(() => session(dir, pr));
  const whyNoAgent = () => (place.error ? String((place.error as Error).message ?? place.error) : undefined);
  const [conversation, setConversation] = createSignal<string>();
  /** Says something in this pull request's session. Resolves with what the agent ends its turn on. */
  const tell = async (prompt: string) => {
    const ready = place.latest;
    if (!ready)
      throw new Error(whyNoAgent() ?? "The agent's folder is not ready yet. Try again in a moment.");
    const id = conversation();
    if (!id) {
      const started = await start(defaultAgent(), ready.cwd, prompt, ready.brief);
      setConversation(started);
      return turnEnd(started, 0);
    }
    await show(id);
    if (conversations[id]?.state.busy) throw new Error('The agent is busy. Wait for it, or stop it.');
    const from = conversations[id]?.entries.length ?? 0;
    await send(id, prompt);
    return turnEnd(id, from);
  };
  const askAgent: Reviewing['askAgent'] = (prompt, about) => {
    const note: Note = { id: crypto.randomUUID(), ...about };
    addNote(pull, note);
    setAgentShown(true);
    tell(prompt).then(
      (answer) =>
        settleNote(pull, note.id, { answer: answer || 'The agent ended its turn without saying anything.' }),
      (err) => settleNote(pull, note.id, { failed: err instanceof Error ? err.message : String(err) }),
    );
  };

  /** The agent's review of the whole pull request: a summary, and comments drafted on lines. */
  const reviewAll = createAction(async (focus: string) => {
    const review = data.review;
    if (!review) return;
    // A new review takes the place of the last one and of the drafts you left from it.
    dropNotes(pull, (note) => Boolean(note.summary) || note.question === FROM_REVIEW);
    const id = crypto.randomUUID();
    addNote(pull, { id, summary: true, question: focus });
    setAgentShown(true);
    try {
      const read = readReview(await tell(reviewPrompt(review, focus)));
      settleNote(pull, id, { answer: read.summary });
      addNotes(
        pull,
        read.comments.map((c) => ({
          id: crypto.randomUUID(),
          place: c.place,
          question: FROM_REVIEW,
          answer: c.text,
        })),
      );
    } catch (err) {
      settleNote(pull, id, { failed: err instanceof Error ? err.message : String(err) });
    }
  });

  const reviewing = (review: Review): Reviewing => ({
    dir,
    pr,
    pull,
    review,
    askAgent,
    reviewAll: (focus) => void reviewAll.run(focus),
    reviewingAll: reviewAll.busy(),
    tell: (prompt) => {
      setAgentShown(true);
      return tell(prompt);
    },
    showFile: (path, at) => setShown({ kind: 'file', path, at }),
  });

  const latest = () => runs.latest?.[0];
  const shownFile = () => {
    const s = shown();
    return s.kind === 'file' ? s : undefined;
  };

  return (
    <Show
      when={data.review}
      fallback={
        <p class="review-pad" classList={{ dim: !failure(), 'note-failed': Boolean(failure()) }}>
          {failure() ?? 'Asking dova'}
        </p>
      }
    >
      {(review) => (
        <div class="review">
          <div class="review-bar">
            <div class="review-name">
              <h1>{review().pr.title}</h1>
              <span class="dim">
                <span class="accent">#{review().pr.id}</span> ·{' '}
                <span class="review-state">{stateOf(review())}</span> · {review().author} ·{' '}
                {review().pr.sourceBranch} into {review().pr.targetBranch}
                <Show when={review().merge === 'conflicts'}>
                  <span class="note-failed"> · conflicts with {review().pr.targetBranch}</span>
                </Show>
              </span>
            </div>
            <span class="top-gap" />
            <Show when={failure()}>{(why) => <span class="note-failed review-late">{why()}</span>}</Show>
            <Show when={latest()}>
              {(run) => (
                <button type="button" class="quiet" onClick={() => setShown({ kind: 'checks' })}>
                  <span class="mark" data-state={outcome(run().result, run().status).state}>
                    {run().name} {outcome(run().result, run().status).words}
                  </span>
                </button>
              )}
            </Show>
            <span class="review-votes dim">
              <For each={review().reviewers.filter((r) => r.vote !== 0)}>
                {(r) => (
                  <span>
                    {r.name}{' '}
                    <span class="mark" data-state={voteWords(r.vote).state}>
                      {voteWords(r.vote).words}
                    </span>
                  </span>
                )}
              </For>
            </span>
            <button
              type="button"
              class="button"
              disabled={reviewAll.busy()}
              onClick={() => {
                setShown({ kind: 'overview' });
                void reviewAll.run('');
              }}
            >
              {reviewAll.busy() ? 'The agent is reviewing' : 'Review with the agent'}
            </button>
            <Decide review={review()} dir={dir} pr={pr} />
            <button
              type="button"
              class="quiet"
              aria-pressed={agentShown()}
              onClick={() => setAgentShown(!agentShown())}
            >
              {agentShown() ? 'Hide the agent' : 'Show the agent'}
            </button>
          </div>

          <div class="review-cols" data-agent={agentShown()}>
            <aside class="review-left" aria-label="Parts of this pull request">
              <button
                type="button"
                class="row review-part"
                aria-current={shown().kind === 'overview' ? 'true' : undefined}
                onClick={() => setShown({ kind: 'overview' })}
              >
                <span class="tree-name">Overview</span>
                <span class="dim">
                  {review().threads.filter((t) => !t.place && t.unresolved).length || ''}
                  {review().threads.some((t) => !t.place && t.unresolved) ? ' open' : ''}
                </span>
              </button>
              <button
                type="button"
                class="row review-part"
                aria-current={shown().kind === 'checks' ? 'true' : undefined}
                onClick={() => setShown({ kind: 'checks' })}
              >
                <span class="tree-name">Checks</span>
                <Show when={latest()} fallback={<span class="dim">{runs.loading ? '' : 'none'}</span>}>
                  {(run) => (
                    <span class="mark" data-state={outcome(run().result, run().status).state}>
                      {outcome(run().result, run().status).words}
                    </span>
                  )}
                </Show>
              </button>
              <Files
                r={reviewing(review())}
                current={shownFile()?.path}
                onPick={(path) => setShown({ kind: 'file', path })}
              />
            </aside>

            <section class="review-middle" aria-label="What you picked">
              <Show when={shown().kind === 'overview'}>
                <Overview r={reviewing(review())} />
              </Show>
              <Show when={shown().kind === 'checks'}>
                <Checks r={reviewing(review())} runs={runs.latest} onChanged={() => void recheck()} />
              </Show>
              <Show when={shownFile()?.path} keyed>
                {(path) => <FileDiff r={reviewing(review())} path={path} at={shownFile()?.at} />}
              </Show>
            </section>

            <Show when={agentShown()}>
              <Show
                when={place.latest}
                fallback={
                  <section class="chat review-nochat" aria-label="The agent">
                    <p classList={{ dim: !place.error, 'note-failed': Boolean(place.error) }}>
                      {whyNoAgent() ?? "Getting the pull request's own folder ready for the agent"}
                    </p>
                  </section>
                }
              >
                {(ready) => (
                  <Chat
                    cwd={ready().cwd}
                    brief={ready().brief}
                    shown={conversation()}
                    onShow={setConversation}
                    onOpenFile={(path) => {
                      // A file the agent touched is in its own checkout. If the pull request changes it, show its diff.
                      const relative = path.startsWith(`${ready().cwd}/`)
                        ? path.slice(ready().cwd.length + 1)
                        : path;
                      if (review().files.some((f) => f.path === relative))
                        setShown({ kind: 'file', path: relative });
                    }}
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

/** Where a pull request stands, in a word or a few. */
function stateOf(review: Review) {
  if (review.pr.isDraft && review.pr.status === 'active') return 'draft';
  if (review.autoComplete && review.pr.status === 'active') return 'active, completes when its checks pass';
  return review.pr.status;
}

/** The two decisions: your vote, and what becomes of the pull request. */
function Decide(props: { review: Review; dir: string; pr: number }) {
  const voting = createAction((value: number) => vote(props.dir, props.pr, value));
  const setting = createAction((to: PullTo) => setPull(props.dir, props.pr, to));
  const mine = () => props.review.vote;
  const activeNow = () => props.review.pr.status === 'active';
  const options = (): Choice[] => {
    const review = props.review;
    if (review.pr.status === 'abandoned')
      return [{ label: 'Make it active again', run: () => setting.run('reactivate') }];
    if (!activeNow()) return [];
    const complete: Choice[] = [
      {
        label: 'Complete now',
        detail: `Squash into ${review.pr.targetBranch}, delete the branch, close the work items`,
        sure: `Yes, complete it into ${review.pr.targetBranch}`,
        run: () => setting.run('complete'),
      },
      review.autoComplete
        ? { label: 'Stop completing by itself', run: () => setting.run('cancelAutoComplete') }
        : {
            label: 'Complete when its checks pass',
            detail: 'Sets auto-complete',
            run: () => setting.run('autoComplete'),
          },
    ];
    return [
      // A draft can't be completed. It is made ready first.
      ...(review.pr.isDraft ? [] : complete),
      review.pr.isDraft
        ? { label: 'Ready for review', detail: 'Takes it out of draft', run: () => setting.run('ready') }
        : { label: 'Put back into draft', run: () => setting.run('draft') },
      { label: 'Abandon', sure: 'Yes, abandon it', run: () => setting.run('abandon') },
    ];
  };
  return (
    <>
      <Show when={voting.error() ?? setting.error()}>
        {(error) => <span class="note-failed review-late">{error()}</span>}
      </Show>
      <Show when={activeNow()}>
        <div class="split">
          <button
            type="button"
            class="button"
            classList={{ 'button-accent': mine() < 10, 'button-done': mine() >= 10 }}
            disabled={voting.busy()}
            title={mine() >= 10 ? 'Click to take your vote back' : undefined}
            onClick={() => void voting.run(mine() >= 10 ? 0 : 10)}
          >
            {mine() >= 10 ? 'Approved' : 'Approve'}
          </button>
          <Menu
            label="Other votes"
            classList={{ 'button-accent': mine() < 10, 'button-done': mine() >= 10 }}
            choices={[
              { label: 'Approve', run: () => voting.run(10) },
              { label: 'Approve with suggestions', run: () => voting.run(5) },
              { label: 'Wait for the author', run: () => voting.run(-5) },
              { label: 'Reject', run: () => voting.run(-10) },
              { label: 'Take my vote back', run: () => voting.run(0) },
            ]}
          />
        </div>
        <Show when={mine() !== 0 && mine() < 10}>
          <span class="mark" data-state={voteWords(mine()).state}>
            You: {voteWords(mine()).words}
          </span>
        </Show>
      </Show>
      <Show when={options().length > 0}>
        <Menu
          label={!activeNow() ? 'Abandoned' : props.review.pr.isDraft ? 'Draft' : 'Complete'}
          words
          choices={options()}
          busy={setting.busy()}
        />
      </Show>
    </>
  );
}

interface Choice {
  label: string;
  detail?: string;
  /** What the choice says once clicked, when it must be clicked twice to be done. */
  sure?: string;
  run: () => unknown;
}

/** A button that opens a short list of things to do. */
function Menu(props: {
  label: string;
  /** Show the label as words. Without it the button is only an arrow. */
  words?: boolean;
  choices: Choice[];
  busy?: boolean;
  classList?: Record<string, boolean>;
}) {
  const [open, setOpen] = createSignal(false);
  const [armed, setArmed] = createSignal<string>();
  let box!: HTMLDivElement;
  const away = (e: PointerEvent) => {
    if (!box.contains(e.target as Node)) setOpen(false);
  };
  createEffect(() => {
    if (!open()) return setArmed();
    document.addEventListener('pointerdown', away);
    onCleanup(() => document.removeEventListener('pointerdown', away));
  });
  return (
    <div class="menu" ref={box}>
      <button
        type="button"
        class="button"
        classList={props.classList}
        aria-label={props.words ? undefined : props.label}
        aria-expanded={open()}
        disabled={props.busy}
        onClick={() => setOpen(!open())}
      >
        {props.words ? `${props.label} ` : ''}▾
      </button>
      <Show when={open()}>
        <div class="menu-list">
          <For each={props.choices}>
            {(choice) => (
              <button
                type="button"
                class="row menu-choice"
                onClick={() => {
                  if (choice.sure && armed() !== choice.label) return setArmed(choice.label);
                  setOpen(false);
                  void choice.run();
                }}
              >
                <span classList={{ 'note-failed': armed() === choice.label }}>
                  {armed() === choice.label ? choice.sure : choice.label}
                </span>
                <Show when={choice.detail && armed() !== choice.label}>
                  <span class="dim">{choice.detail}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
