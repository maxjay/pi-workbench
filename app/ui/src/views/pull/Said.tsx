// What is said about one thing: a review thread, or lines no thread is on
// yet. It is one chat. In it, in the order they were said: comments other
// people see, what you asked the agent, and what it answered. Ask is a
// switch on the line you write in: on, Enter goes to the agent; off, Enter
// posts. Shut, it is one line.

import { createSignal, For, type JSX, Show } from 'solid-js';
import { createAction } from '../../components/action';
import { Markdown } from '../../components/Markdown';
import { shutOnAway } from '../../components/shut';
import type { Comment } from '../../protocol/Comment';
import type { Place } from '../../protocol/Place';
import type { Thread } from '../../protocol/Thread';
import {
  addTalk,
  comment,
  dropTalk,
  patchTalk,
  reply,
  setThread,
  type Talk,
  THREAD_STATES,
  talkFor,
} from '../../state/review';
import { type Pull, placeName, samePlace, when } from './parts';
import type { Host } from './talker';

interface Line {
  key: string;
  who: string;
  /** Only you see it. */
  yours: boolean;
  agent: boolean;
  at: number;
  when: string;
  text?: string;
  failed?: string;
}

/** A thread's state, in the word shown for it. */
export function stateLabel(status: string) {
  if (status === 'byDesign') return 'By design';
  return THREAD_STATES.find((s) => s.status === status)?.label ?? status;
}

export function Said(props: {
  /** The pull request it is in, or whatever else it is said about. */
  p: Pull | Host;
  /** With no thread: the comments already posted, how one more is posted, and the number its talk is kept under. */
  posted?: Comment[];
  post?: (words: string) => Promise<unknown>;
  id?: number;
  /** Always open, with nothing to shut or drop: it is the whole of a discussion. */
  plain?: boolean;
  /** The review thread, when there is one. */
  thread?: Thread;
  /** What you and the agent said about it, when anything was. */
  talk?: Talk;
  /** The lines it is on. Null for one about the whole pull request. */
  place: Place | null;
  open?: boolean;
  onOpen?: () => void;
  onClose?: () => void;
  /** Says where it is, when it isn't drawn under its lines. */
  showPlace?: boolean;
  /** A push took away the lines it was on. */
  gone?: boolean;
  /** The pointer is on it, or has left it. */
  onLight?: (on: boolean) => void;
  /** What was said only to you is now about a thread that was just posted. */
  onThread?: (thread: number) => void;
}) {
  const p = () => props.p;
  /** The pull request, for what only a pull request's threads do. */
  const pull = () => props.p as Pull;
  const [text, setText] = createSignal('');
  const [states, setStates] = createSignal(false);
  let statesBox: HTMLSpanElement | undefined;
  shutOnAway(
    states,
    () => statesBox,
    () => setStates(false),
  );
  const posted = () => props.thread?.comments ?? props.posted ?? [];
  const resolved = () => (props.thread ? !props.thread.unresolved : Boolean(props.talk?.resolved));
  /** What the agent's review found. On your own pull request it is only ever talked about. */
  const finding = () => !props.thread && Boolean(props.talk?.finding);
  const open = () => props.plain || props.open;
  // Enter goes to the agent while nothing is posted, and to the thread once something is. Your own switch wins.
  const asking = () => finding() || (props.talk?.ask ?? (!props.thread && !props.plain));
  /** The talk's id, made on first use: a thread has none until you ask something. */
  const talkId = () => {
    if (props.talk) return props.talk.id;
    if (props.thread) return talkFor(p().key, props.thread.id);
    if (props.id !== undefined) return talkFor(p().key, props.id);
    return addTalk(p().key, { place: props.place ?? undefined, said: [] });
  };

  const lines = (): Line[] => {
    const theirs: Line[] = posted().map((c, i) => ({
      key: `c${c.id}`,
      who: c.author ?? 'Someone',
      yours: false,
      agent: false,
      // A comment with no time keeps its place by its order.
      at: c.publishedAt ? Date.parse(c.publishedAt) : i,
      when: when(c.publishedAt),
      text: c.text,
    }));
    const mine: Line[] = (props.talk?.said ?? []).map((s) => ({
      key: s.id,
      who: s.who === 'agent' ? p().agent() : 'You',
      yours: true,
      agent: s.who === 'agent',
      at: s.at,
      when: '',
      text: s.text,
      failed: s.failed,
    }));
    return [...theirs, ...mine].sort((a, b) => a.at - b.at);
  };
  /** What its one line shows: a thread's first comment, or the first thing said about lines. */
  const first = () => lines().find((line) => !line.yours) ?? lines()[0];
  const where = () => (props.place ? placeName(props.place) : 'the pull request');

  /** Posts a comment: a reply in the thread, or a new thread on the lines. */
  const posting = createAction(async (words: string) => {
    if (props.post) return props.post(words);
    if (props.thread) return reply(pull().dir, pull().pr, props.thread.id, words);
    const [at, talk] = [props.place, props.talk];
    await comment(pull().dir, pull().pr, words, at);
    await pull().reload();
    if (!talk) return;
    // What you and the agent said about the lines now belongs to the thread that was made.
    const made = [...pull().review.threads]
      .reverse()
      .find(
        (t) =>
          t.comments[0]?.text.trim() === words &&
          (at ? t.place !== null && samePlace(t.place, at) : t.place === null),
      );
    if (!made || talk.said.length === 0) return dropTalk(p().key, talk.id);
    patchTalk(p().key, talk.id, {
      thread: made.id,
      place: undefined,
      finding: false,
      ask: undefined,
      draft: undefined,
    });
    props.onThread?.(made.id);
  });
  const stating = createAction((status: string) =>
    setThread(pull().dir, pull().pr, (props.thread as Thread).id, status),
  );

  const send = () => {
    const words = text().trim();
    if (!words || posting.busy()) return;
    setText('');
    if (asking()) p().ask(talkId(), words);
    else void posting.run(words);
  };
  const draft = () => props.talk?.draft;
  const postDraft = async () => {
    const words = draft()?.text.trim();
    const id = props.talk?.id;
    if (!words || !id || posting.busy()) return;
    await posting.run(words);
    if (!posting.error()) patchTalk(p().key, id, { draft: undefined });
  };

  /** What you can do to the whole of it. They are at the right of its first line. */
  const Controls = (): JSX.Element => (
    <>
      <Show when={props.thread}>
        {(thread) => (
          <span class="said-menu" ref={statesBox}>
            <button
              type="button"
              class="quiet said-state"
              data-open={thread().unresolved}
              aria-expanded={states()}
              disabled={stating.busy()}
              onClick={() => setStates(!states())}
            >
              {stateLabel(thread().status)} ▾
            </button>
            <Show when={states()}>
              <div class="said-states">
                <For each={THREAD_STATES}>
                  {(state) => (
                    <button
                      type="button"
                      class="row"
                      onClick={() => {
                        setStates(false);
                        void stating.run(state.status);
                      }}
                    >
                      <span
                        class="sq"
                        data-tone={
                          state.status === 'active' || state.status === 'pending' ? 'people' : 'faint'
                        }
                      />
                      {state.label}
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </span>
        )}
      </Show>
      <Show when={!props.thread && props.talk}>
        {(talk) => (
          <Show
            when={finding()}
            fallback={
              <button type="button" class="quiet" onClick={() => dropTalk(p().key, talk().id)}>
                Discard
              </button>
            }
          >
            {/* On someone else's pull request, what the agent found can become a comment. */}
            <Show when={!pull().review.mine && !draft()}>
              <button
                type="button"
                class="quiet"
                onClick={() =>
                  patchTalk(p().key, talk().id, {
                    draft: { text: talk().said.find((s) => s.who === 'agent')?.text ?? '', writing: false },
                  })
                }
              >
                Comment
              </button>
            </Show>
            <Show when={props.place}>
              {(at) => (
                <button
                  type="button"
                  class="quiet"
                  title="Go on with this in the chat on the right"
                  onClick={() => pull().toChat(at())}
                >
                  To chat
                </button>
              )}
            </Show>
            <button
              type="button"
              class="quiet"
              onClick={() => {
                const done = !talk().resolved;
                patchTalk(p().key, talk().id, { resolved: done });
                if (done) props.onClose?.();
              }}
            >
              {talk().resolved ? 'Reopen' : 'Resolve'}
            </button>
          </Show>
        )}
      </Show>
      <button type="button" class="quiet" onClick={props.onClose}>
        Close
      </button>
    </>
  );

  return (
    <div
      class="said"
      data-kind={props.plain ? 'plain' : resolved() ? 'done' : props.thread ? 'thread' : 'yours'}
      onPointerEnter={() => props.onLight?.(true)}
      onPointerLeave={() => props.onLight?.(false)}
    >
      <Show
        when={open()}
        fallback={
          <button type="button" class="said-peek" onClick={props.onOpen}>
            <span class="said-who" data-agent={first()?.agent || undefined}>
              {first()?.who ?? 'You'}
            </span>
            <span class="said-first">{first()?.text ?? first()?.failed ?? (first() ? 'working' : '')}</span>
            <span class="said-more">
              {[
                props.showPlace && props.place ? placeName(props.place) : '',
                props.gone ? 'lines gone' : '',
                lines().length > 1 ? String(lines().length) : '',
                props.thread
                  ? resolved()
                    ? stateLabel(props.thread.status).toLowerCase()
                    : ''
                  : resolved()
                    ? 'resolved'
                    : 'only you',
              ]
                .filter(Boolean)
                .join(' · ')}
            </span>
          </button>
        }
      >
        <div class="said-open">
          <Show when={props.plain && lines().length === 0 && !draft()}>
            <span class="dim">Nothing said yet.</span>
          </Show>
          <Show when={!props.plain && (props.showPlace || lines().length === 0)}>
            <div class="said-by">
              <span>
                {where()}
                {props.gone ? ' · lines gone' : ''}
                {!props.thread && asking() ? ' · only you' : ''}
              </span>
              <Show when={lines().length === 0}>
                <span class="top-gap" />
                <Controls />
              </Show>
            </div>
          </Show>

          <For each={lines()}>
            {(line, at) => (
              <div class="said-line" data-yours={line.yours || undefined}>
                <div class="said-by">
                  <span>
                    <span
                      class="said-who"
                      data-agent={line.agent || undefined}
                      data-you={(line.yours && !line.agent) || undefined}
                    >
                      {line.who}
                    </span>
                    {line.yours ? ' · only you' : line.when ? ` · ${line.when}` : ''}
                  </span>
                  <Show when={at() === 0 && !props.plain}>
                    <span class="top-gap" />
                    <Controls />
                  </Show>
                </div>
                <Show
                  when={line.text !== undefined}
                  fallback={
                    <Show when={line.failed} fallback={<span class="said-working">working</span>}>
                      {(why) => <span class="note-failed">{why()}</span>}
                    </Show>
                  }
                >
                  <div class="said-text" data-you={(line.yours && !line.agent) || undefined}>
                    <Markdown text={line.text ?? ''} />
                  </div>
                </Show>
              </div>
            )}
          </For>

          <Show when={draft()}>
            {(d) => (
              <div class="said-draft">
                <span class="said-label">
                  {d().writing ? `${p().agent()} is writing` : `Draft from ${p().agent()} · not posted`}
                </span>
                <Show when={!d().writing}>
                  <textarea
                    class="said-input"
                    aria-label="The draft"
                    rows={Math.min(
                      10,
                      Math.max(1, d().text.split('\n').length, Math.ceil(d().text.length / 70)),
                    )}
                    value={d().text}
                    onInput={(e) =>
                      props.talk &&
                      patchTalk(p().key, props.talk.id, {
                        draft: { text: e.currentTarget.value, writing: false },
                      })
                    }
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter' || e.shiftKey) return;
                      e.preventDefault();
                      void postDraft();
                    }}
                  />
                  <div class="said-draft-foot">
                    <button
                      type="button"
                      class="quiet"
                      onClick={() => props.talk && patchTalk(p().key, props.talk.id, { draft: undefined })}
                    >
                      Discard
                    </button>
                    <button
                      type="button"
                      class="soft"
                      data-tone="agent"
                      data-solid
                      title="Enter"
                      disabled={posting.busy()}
                      onClick={() => void postDraft()}
                    >
                      Post
                    </button>
                  </div>
                </Show>
              </div>
            )}
          </Show>

          <Show when={posting.error() ?? stating.error()}>
            {(error) => <p class="note-failed">{error()}</p>}
          </Show>
          <div class="said-foot">
            <textarea
              class="said-input"
              data-ask={asking() || undefined}
              rows={Math.min(6, Math.max(1, text().split('\n').length, Math.ceil(text().length / 64)))}
              placeholder={asking() ? `Ask ${p().agent()}` : props.thread ? 'Reply' : 'Comment'}
              aria-label={asking() ? `Ask ${p().agent()}` : props.thread ? 'Reply' : 'Comment'}
              value={text()}
              onInput={(e) => setText(e.currentTarget.value)}
              onKeyDown={(e) => {
                // Tab switches who Enter goes to.
                if (e.key === 'Tab' && !e.shiftKey && !finding()) {
                  e.preventDefault();
                  patchTalk(p().key, talkId(), { ask: !asking() });
                  return;
                }
                if (e.key === 'Escape' && !props.plain) props.onClose?.();
                if (e.key !== 'Enter' || e.shiftKey) return;
                e.preventDefault();
                send();
              }}
              ref={(el) => props.plain || requestAnimationFrame(() => el.focus({ preventScroll: true }))}
            />
            <Show when={!finding()}>
              <button
                type="button"
                class="quiet said-drafter"
                disabled={Boolean(draft()?.writing)}
                title={`${p().agent()} writes the comment, for you to change and post`}
                onClick={() => p().draft(talkId())}
              >
                Draft
              </button>
              <button
                type="button"
                class="quiet said-ask"
                aria-pressed={asking()}
                title="Tab"
                onClick={() => patchTalk(p().key, talkId(), { ask: !asking() })}
              >
                Ask
              </button>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
